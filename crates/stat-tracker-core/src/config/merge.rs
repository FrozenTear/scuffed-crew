//! The general in-place save behind [`Config::text_for_save`]: every key a
//! save changes is edited in the existing document, at the top level and
//! inside tables (`[sync]`, `[auto_detect]`), so comments, key order and the
//! keys the struct does not know stay as they were.
//!
//! The diff is between what the file parses as and `next`, both serialized,
//! so a key the struct does not know is never touched, and a key the struct
//! knows but neither side changes is left byte for byte (a token-less
//! `[sync]` table reads as sync off on both sides, so its `server_url` stays).
//! A changed key keeps its own comments: the value is replaced and its
//! decoration (spacing and a trailing comment) is copied over. A new key goes
//! at the end of its table. A key the struct no longer writes (an `Option`
//! that is now `None`, or a default the format leaves out) is removed, with
//! the comment lines directly above it.

use toml_edit::{DocumentMut, Item, Table, TableLike};

use super::Config;

/// `existing` with every key that differs between `loaded` (what `existing`
/// parses as) and `next` set, inserted or removed in place.
///
/// `None` when the document cannot be edited in place (a key the struct
/// knows has a shape the edit does not expect) or when the edited text
/// would not read back as exactly `next`. The caller then writes a full
/// document, which is what every save did before this edit existed.
pub(super) fn edit_in_place(existing: &str, loaded: &Config, next: &Config) -> Option<String> {
    let was = toml::Table::try_from(loaded).ok()?;
    let now = toml::Table::try_from(next).ok()?;
    // `toml::Table` is sorted by key; the parsed text keeps field order,
    // so new keys go in the order a full save would write them.
    let order: DocumentMut = toml::to_string(next).ok()?.parse().ok()?;
    let mut doc: DocumentMut = existing.parse().ok()?;
    edit_table(doc.as_table_mut(), &was, &now, order.as_table())?;
    let text = doc.to_string();
    let reread = toml::from_str::<Config>(&text).ok()?;
    (reread == *next).then_some(text)
}

fn edit_table(
    doc: &mut dyn TableLike,
    was: &toml::Table,
    now: &toml::Table,
    order: &dyn TableLike,
) -> Option<()> {
    for (key, old) in was {
        if !now.contains_key(key) {
            remove_known(doc, key, old);
        }
    }
    for (key, fresh) in order.iter() {
        let value = now.get(key)?;
        if was.get(key) == Some(value) {
            continue;
        }
        set_key(doc, key, was.get(key), value, fresh)?;
    }
    Some(())
}

/// Remove what the struct wrote under `key`. A table loses only the keys
/// the struct knows; it goes when nothing else is left in it.
fn remove_known(doc: &mut dyn TableLike, key: &str, old: &toml::Value) {
    if let toml::Value::Table(known) = old
        && let Some(sub) = doc.get_mut(key).and_then(Item::as_table_like_mut)
    {
        for (sub_key, sub_old) in known {
            remove_known(sub, sub_key, sub_old);
        }
        if !sub.is_empty() {
            return;
        }
    }
    doc.remove(key);
}

fn set_key(
    doc: &mut dyn TableLike,
    key: &str,
    old: Option<&toml::Value>,
    value: &toml::Value,
    fresh: &Item,
) -> Option<()> {
    let Some(item) = doc.get_mut(key) else {
        doc.insert(key, clean(fresh)?);
        return Some(());
    };
    if let toml::Value::Table(now) = value
        && item.is_table_like()
    {
        let none = toml::Table::new();
        let was = match old {
            Some(toml::Value::Table(was)) => was,
            _ => &none,
        };
        return edit_table(item.as_table_like_mut()?, was, now, fresh.as_table_like()?);
    }
    // `Table::insert` on an existing key resets the key's decoration (the
    // comments above it), so the value is replaced where it is instead.
    let current = item.as_value_mut()?;
    let mut replacement = fresh.as_value()?.clone();
    *replacement.decor_mut() = current.decor().clone();
    *current = replacement;
    Some(())
}

/// `fresh` with default formatting, for a key or table the file lacks.
fn clean(fresh: &Item) -> Option<Item> {
    match fresh {
        Item::Value(value) => {
            let mut value = value.clone();
            value.decor_mut().clear();
            Some(Item::Value(value))
        }
        Item::Table(table) => {
            let mut out = Table::new();
            for (key, item) in table.iter() {
                out.insert(key, clean(item)?);
            }
            Some(Item::Table(out))
        }
        _ => None,
    }
}
