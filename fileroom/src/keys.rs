// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use slpc::toml_edit::{Item, TableLike, Value};

use crate::conventions::{Agent, Date, Hash, Identifier, Instant, Temporal};
use crate::Malformed;

/// Typed access to a TOML table, with each failure naming the key and the rule.
pub(crate) struct Keys<'a> {
    table: &'a dyn TableLike,
    at: String,
}

impl<'a> Keys<'a> {
    pub(crate) fn new(table: &'a dyn TableLike, at: impl Into<String>) -> Self {
        Self {
            table,
            at: at.into(),
        }
    }

    pub(crate) fn path(&self, key: &str) -> String {
        if self.at.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.at)
        }
    }

    pub(crate) fn has(&self, key: &str) -> bool {
        self.table.get(key).is_some()
    }

    fn fail(&self, rule: &'static str, key: &str, problem: impl Into<String>) -> Malformed {
        Malformed::new(rule, self.path(key), problem)
    }

    pub(crate) fn required_item(
        &self,
        rule: &'static str,
        key: &str,
    ) -> Result<&'a Item, Malformed> {
        self.required(rule, key)
    }

    fn required(&self, rule: &'static str, key: &str) -> Result<&'a Item, Malformed> {
        self.table
            .get(key)
            .ok_or_else(|| self.fail(rule, key, "required and absent"))
    }

    pub(crate) fn string(&self, rule: &'static str, key: &str) -> Result<&'a str, Malformed> {
        self.required(rule, key)?
            .as_str()
            .ok_or_else(|| self.fail(rule, key, "not a string"))
    }

    pub(crate) fn nonempty(&self, rule: &'static str, key: &str) -> Result<&'a str, Malformed> {
        let s = self.string(rule, key)?;
        if s.is_empty() {
            return Err(self.fail(rule, key, "empty"));
        }
        Ok(s)
    }

    pub(crate) fn optional_string(
        &self,
        rule: &'static str,
        key: &str,
    ) -> Result<Option<&'a str>, Malformed> {
        self.table
            .get(key)
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| self.fail(rule, key, "not a string"))
            })
            .transpose()
    }

    pub(crate) fn one_of(
        &self,
        rule: &'static str,
        key: &str,
        allowed: &[&str],
    ) -> Result<&'a str, Malformed> {
        let s = self.string(rule, key)?;
        if allowed.contains(&s) {
            Ok(s)
        } else {
            Err(self.fail(
                rule,
                key,
                format!("{s:?} is not one of {}", allowed.join(", ")),
            ))
        }
    }

    pub(crate) fn integer(&self, rule: &'static str, key: &str) -> Result<i64, Malformed> {
        self.required(rule, key)?
            .as_integer()
            .ok_or_else(|| self.fail(rule, key, "not an integer"))
    }

    pub(crate) fn size(&self, rule: &'static str, key: &str) -> Result<u64, Malformed> {
        u64::try_from(self.integer(rule, key)?).map_err(|_| self.fail(rule, key, "negative"))
    }

    pub(crate) fn boolean(&self, rule: &'static str, key: &str) -> Result<bool, Malformed> {
        self.required(rule, key)?
            .as_bool()
            .ok_or_else(|| self.fail(rule, key, "not a boolean"))
    }

    pub(crate) fn identifier(
        &self,
        rule: &'static str,
        key: &str,
    ) -> Result<Identifier, Malformed> {
        Identifier::parse(self.string(rule, key)?).map_err(|e| self.fail("CONVENTIONS 1", key, e))
    }

    pub(crate) fn hash(&self, rule: &'static str, key: &str) -> Result<Hash, Malformed> {
        Hash::parse(self.string(rule, key)?).map_err(|e| self.fail("CONVENTIONS 3", key, e))
    }

    fn temporal(&self, rule: &'static str, key: &str) -> Result<Temporal, Malformed> {
        let dt = self
            .required(rule, key)?
            .as_datetime()
            .ok_or_else(|| self.fail(rule, key, "not a date or date-time"))?;
        Ok(Temporal::of(dt))
    }

    pub(crate) fn date(&self, rule: &'static str, key: &str) -> Result<Date, Malformed> {
        match self.temporal(rule, key)? {
            Temporal::Date(d) => Ok(d),
            Temporal::Instant(_) | Temporal::Unzoned => {
                Err(self.fail(rule, key, "a date-time, not a date"))
            }
            Temporal::LocalTime => Err(self.fail(rule, key, "a time, not a date")),
        }
    }

    pub(crate) fn optional_date(
        &self,
        rule: &'static str,
        key: &str,
    ) -> Result<Option<Date>, Malformed> {
        if self.has(key) {
            self.date(rule, key).map(Some)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn instant(&self, rule: &'static str, key: &str) -> Result<Instant, Malformed> {
        match self.temporal(rule, key)? {
            Temporal::Instant(i) => Ok(i),
            Temporal::Date(_) => Err(self.fail(rule, key, "a date, not an instant")),
            Temporal::LocalTime => Err(self.fail(rule, key, "a time, not an instant")),
            Temporal::Unzoned => Err(self.fail(
                "CONVENTIONS 4",
                key,
                "an instant is an offset date-time in UTC, written with Z",
            )),
        }
    }

    pub(crate) fn table(&self, rule: &'static str, key: &str) -> Result<Keys<'a>, Malformed> {
        let t = self
            .required(rule, key)?
            .as_table_like()
            .ok_or_else(|| self.fail(rule, key, "not a table"))?;
        Ok(Keys::new(t, self.path(key)))
    }

    pub(crate) fn optional_table(
        &self,
        rule: &'static str,
        key: &str,
    ) -> Result<Option<Keys<'a>>, Malformed> {
        if self.has(key) {
            self.table(rule, key).map(Some)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn agent(&self, rule: &'static str, key: &str) -> Result<Agent, Malformed> {
        let t = self.table(rule, key)?;
        Ok(Agent {
            email: t.string("CONVENTIONS 2", "email")?.to_owned(),
            id: t.nonempty("CONVENTIONS 2", "id")?.to_owned(),
        })
    }

    /// An array of tables, whether written as `[[key]]` or inline; absent is empty.
    pub(crate) fn tables(&self, rule: &'static str, key: &str) -> Result<Vec<Keys<'a>>, Malformed> {
        let Some(item) = self.table.get(key) else {
            return Ok(Vec::new());
        };
        let at = |i: usize| format!("{}[{i}]", self.path(key));
        if let Some(array) = item.as_array_of_tables() {
            return Ok(array
                .iter()
                .enumerate()
                .map(|(i, t)| Keys::new(t, at(i)))
                .collect());
        }
        let values = item
            .as_array()
            .ok_or_else(|| self.fail(rule, key, "not an array of tables"))?;
        values
            .iter()
            .enumerate()
            .map(|(i, v)| match v {
                Value::InlineTable(t) => Ok(Keys::new(t, at(i))),
                _ => Err(self.fail(rule, key, "not an array of tables")),
            })
            .collect()
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = (&'a str, &'a Item)> {
        self.table.iter()
    }
}
