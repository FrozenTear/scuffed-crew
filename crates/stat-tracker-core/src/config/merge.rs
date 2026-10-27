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
//! that is now `None`, or a default the format leaves out) is removed with
//! its own line, a comment at the end of that line included. The comment
//! lines directly above it move onto the key or table header that follows
//! it in the same table. They go with it only when nothing there can take
//! them: it was the last entry of its table, or the next entry is a dotted
//! key (`a.b = 1`), an implicit table (`[foo.bar]` with no `[foo]` header)
//! or an array of tables (`[[foo]]`).

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
    remove_keeping_comments(doc, key);
}

/// Remove `key` from `doc` and put the comment lines above it in front of
/// the next key or table header of `doc`, so they stay where they were.
/// With no next entry, or one with no line of its own to carry them (a
/// dotted key, an implicit table, an array of tables), they are dropped.
fn remove_keeping_comments(doc: &mut dyn TableLike, key: &str) {
    let above = leading(doc, key).filter(|text| text.contains('#'));
    let next = doc
        .iter()
        .map(|(k, _)| k)
        .skip_while(|k| *k != key)
        .nth(1)
        .map(str::to_owned);
    doc.remove(key);
    if let (Some(above), Some(next)) = (above, next) {
        let below = leading(doc, &next).unwrap_or_default();
        set_leading(doc, &next, format!("{above}{below}"));
    }
}

/// The text before `key`'s line: a table's header decor, else the key's.
fn leading(doc: &dyn TableLike, key: &str) -> Option<String> {
    let decor = match doc.get(key)? {
        Item::Table(table) => table.decor(),
        Item::Value(_) => doc.key(key)?.leaf_decor(),
        _ => return None,
    };
    decor.prefix()?.as_str().map(str::to_owned)
}

/// Set the text before `key`'s line, where [`leading`] reads it. A table
/// that is not written as its own header (dotted or implicit) has no such
/// text, so it is left alone.
fn set_leading(doc: &mut dyn TableLike, key: &str, text: String) {
    match doc.get_mut(key) {
        Some(Item::Table(table)) if !table.is_implicit() && !table.is_dotted() => {
            table.decor_mut().set_prefix(text);
        }
        Some(Item::Value(_)) => {
            if let Some(mut k) = doc.key_mut(key) {
                k.leaf_decor_mut().set_prefix(text);
            }
        }
        _ => {}
    }
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
