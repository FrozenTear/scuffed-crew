# shellcheck shell=bash
# Append a generated secret, or replace it when the key is present but blank.
# A non-blank value is left alone. ENCRYPTION_KEY, SURREALDB_PASSWORD, and
# ALLOWED_ORIGINS are not passed here: a new encryption key would brick
# sealed data, a new root password would desync the live user, and a blank
# ALLOWED_ORIGINS is intentional.
#
# stdout is only "Appended KEY" or "Replaced blank KEY". The value is not printed.
# The file mode is set to 0600 on every call.
#
# A rewrite passes the new value to awk through the environment, not argv.
# The temporary file is created in the secrets file's own directory, mode 0600,
# and renamed over the original. It is not written under /tmp.

# 0 when the key is missing or every value for it is blank.
secret_needs_value() {
    local file="$1" key="$2" line raw trimmed found
    if [[ ! -f "$file" ]]; then
        return 0
    fi
    if ! grep -q "^${key}=" "$file"; then
        return 0
    fi
    found=0
    while IFS= read -r line || [[ -n "$line" ]]; do
        if [[ "$line" == "${key}="* ]]; then
            found=1
            raw="${line#"${key}="}"
            trimmed="${raw#"${raw%%[![:space:]]*}"}"
            trimmed="${trimmed%"${trimmed##*[![:space:]]}"}"
            if [[ -n "$trimmed" ]]; then
                return 1
            fi
        fi
    done < "$file"
    if [[ "$found" -eq 0 ]]; then
        return 0
    fi
    return 0
}

upsert_generated_secret() {
    local file="$1" key="$2" value="$3"
    local line raw trimmed nonempty tmp
    if [[ ! -f "$file" ]]; then
        echo "error: missing secrets file" >&2
        return 1
    fi
    if [[ ! "$key" =~ ^[A-Za-z0-9_]+$ ]]; then
        echo "error: invalid secret key name" >&2
        return 1
    fi
    if [[ -z "$value" || "$value" == *$'\n'* ]]; then
        echo "error: refusing to write an empty secret for ${key}" >&2
        return 1
    fi
    if grep -q "^${key}=" "$file"; then
        nonempty=0
        while IFS= read -r line || [[ -n "$line" ]]; do
            if [[ "$line" == "${key}="* ]]; then
                raw="${line#"${key}="}"
                trimmed="${raw#"${raw%%[![:space:]]*}"}"
                trimmed="${trimmed%"${trimmed##*[![:space:]]}"}"
                if [[ -n "$trimmed" ]]; then
                    nonempty=1
                fi
            fi
        done < "$file"
        if [[ "$nonempty" -eq 1 ]]; then
            chmod 600 "$file"
            return 0
        fi
        dir="$(dirname "$file")"
        tmp="$(mktemp "${dir}/.secrets-rewrite.XXXXXX")"
        chmod 600 "$tmp"
        # ENVIRON, not -v: -v puts the secret on awk's command line.
        if ! SCUFFED_SECRET_KEY="$key" SCUFFED_SECRET_VALUE="$value" awk '
            BEGIN {
                key = ENVIRON["SCUFFED_SECRET_KEY"]
                value = ENVIRON["SCUFFED_SECRET_VALUE"]
                if (key == "" || value == "") exit 2
                done = 0
                blank = 0
            }
            $0 ~ ("^" key "=") {
                if (!done) {
                    print key "=" value
                    done = 1
                    blank = 0
                }
                next
            }
            $0 ~ /^[[:space:]]*$/ {
                if (blank) next
                blank = 1
                print ""
                next
            }
            { blank = 0; print }
            END { if (!done) print key "=" value }
        ' "$file" > "$tmp"; then
            rm -f "$tmp"
            echo "error: failed to rewrite secrets file for ${key}" >&2
            return 1
        fi
        mv -f "$tmp" "$file"
        chmod 600 "$file"
        echo "Replaced blank ${key}"
        return 0
    fi
    printf '%s=%s\n' "$key" "$value" >> "$file"
    chmod 600 "$file"
    echo "Appended ${key}"
}
