//! Test suites for the prompts and tool descriptions of a server.
//!
//! A case holds an input, as a user would write it, and what should happen: the model calls a given
//! tool, calls no tool, or answers with some text. A suite belongs to one server and may hold the
//! system prompt that is tested together with the tool descriptions. Running suites is done by the
//! variant comparison; this module only stores them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{new_id, now_ms, Db, DbError, DbResult};

/// What should happen for a case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expectation {
    /// The model calls this tool first.
    Tool { name: String },
    /// The model answers without calling a tool.
    NoTool,
    /// The model answers with text that contains this (ignoring case).
    Answer { contains: String },
}

impl Expectation {
    /// The expectation in words, for messages and prompts.
    pub fn describe(&self) -> String {
        match self {
            Expectation::Tool { name } => format!("the model calls the tool {name}"),
            Expectation::NoTool => "the model calls no tool".to_owned(),
            Expectation::Answer { contains } => {
                format!("the model answers with text containing {contains:?}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestCase {
    pub id: String,
    pub input: String,
    pub expectation: Expectation,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestSuite {
    pub id: String,
    pub server_id: String,
    pub name: String,
    /// The system prompt that is tested together with the tool descriptions.
    pub system_prompt: Option<String>,
    pub cases: Vec<TestCase>,
    #[specta(type = u32)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseInput {
    /// The id of an existing case; new cases have none.
    pub id: Option<String>,
    pub input: String,
    pub expectation: Expectation,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestSuiteInput {
    pub server_id: String,
    pub name: String,
    pub system_prompt: Option<String>,
    pub cases: Vec<TestCaseInput>,
}

fn clean(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

impl TestSuiteInput {
    /// Trims text and rejects empty names, inputs, and expectations.
    pub fn normalized(mut self) -> DbResult<Self> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Err(DbError::Invalid("a suite needs a name".into()));
        }
        self.system_prompt = clean(self.system_prompt);
        for (index, case) in self.cases.iter_mut().enumerate() {
            let number = index + 1;
            case.input = case.input.trim().to_owned();
            if case.input.is_empty() {
                return Err(DbError::Invalid(format!("case {number} has no input")));
            }
            case.notes = clean(case.notes.take());
            match &mut case.expectation {
                Expectation::Tool { name } => {
                    *name = name.trim().to_owned();
                    if name.is_empty() {
                        return Err(DbError::Invalid(format!(
                            "case {number} expects a tool but names none"
                        )));
                    }
                }
                Expectation::Answer { contains } => {
                    *contains = contains.trim().to_owned();
                    if contains.is_empty() {
                        return Err(DbError::Invalid(format!(
                            "case {number} expects an answer but says nothing about it"
                        )));
                    }
                }
                Expectation::NoTool => {}
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct TestSuites {
    db: Db,
}

impl TestSuites {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    async fn cases(&self, suite_id: &str) -> DbResult<Vec<TestCase>> {
        let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, input, expectation, notes FROM test_cases WHERE suite_id = ? ORDER BY position",
        )
        .bind(suite_id)
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter()
            .map(|(id, input, expectation, notes)| {
                let expectation = serde_json::from_str(&expectation)
                    .map_err(|e| DbError::Invalid(format!("corrupt test case {id}: {e}")))?;
                Ok(TestCase {
                    id,
                    input,
                    expectation,
                    notes,
                })
            })
            .collect()
    }

    pub async fn get(&self, id: &str) -> DbResult<TestSuite> {
        let row: Option<(String, String, Option<String>, i64)> = sqlx::query_as(
            "SELECT server_id, name, system_prompt, updated_at FROM test_suites WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await?;
        let (server_id, name, system_prompt, updated_at) =
            row.ok_or_else(|| DbError::NotFound(format!("test suite {id}")))?;
        Ok(TestSuite {
            id: id.to_owned(),
            server_id,
            name,
            system_prompt,
            cases: self.cases(id).await?,
            updated_at,
        })
    }

    /// The suites of a server, by name.
    pub async fn list(&self, server_id: &str) -> DbResult<Vec<TestSuite>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM test_suites WHERE server_id = ? ORDER BY name COLLATE NOCASE, id",
        )
        .bind(server_id)
        .fetch_all(self.db.pool())
        .await?;
        let mut suites = Vec::with_capacity(ids.len());
        for id in ids {
            suites.push(self.get(&id).await?);
        }
        Ok(suites)
    }

    /// Creates a suite (`id` is `None`) or replaces an existing one with all its cases.
    pub async fn save(&self, id: Option<&str>, input: TestSuiteInput) -> DbResult<TestSuite> {
        let input = input.normalized()?;
        let server_exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM servers WHERE id = ?")
            .bind(&input.server_id)
            .fetch_optional(self.db.pool())
            .await?;
        if server_exists.is_none() {
            return Err(DbError::NotFound(format!("server {}", input.server_id)));
        }
        let suite_id = match id {
            Some(existing) => {
                // The suite stays with its server.
                let owner: Option<String> =
                    sqlx::query_scalar("SELECT server_id FROM test_suites WHERE id = ?")
                        .bind(existing)
                        .fetch_optional(self.db.pool())
                        .await?;
                match owner {
                    None => return Err(DbError::NotFound(format!("test suite {existing}"))),
                    Some(owner) if owner != input.server_id => {
                        return Err(DbError::Invalid(
                            "a suite cannot move to another server".into(),
                        ))
                    }
                    Some(_) => existing.to_owned(),
                }
            }
            None => new_id(),
        };
        let mut tx = self.db.pool().begin().await?;
        if id.is_some() {
            sqlx::query(
                "UPDATE test_suites SET name = ?, system_prompt = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&input.name)
            .bind(&input.system_prompt)
            .bind(now_ms())
            .bind(&suite_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM test_cases WHERE suite_id = ?")
                .bind(&suite_id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query(
                "INSERT INTO test_suites (id, server_id, name, system_prompt, updated_at) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&suite_id)
            .bind(&input.server_id)
            .bind(&input.name)
            .bind(&input.system_prompt)
            .bind(now_ms())
            .execute(&mut *tx)
            .await?;
        }
        for (position, case) in input.cases.iter().enumerate() {
            let expectation = serde_json::to_string(&case.expectation)
                .map_err(|e| DbError::Invalid(format!("could not save the case: {e}")))?;
            sqlx::query(
                "INSERT INTO test_cases (id, suite_id, position, input, expectation, notes) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(case.id.clone().unwrap_or_else(new_id))
            .bind(&suite_id)
            .bind(i64::try_from(position).unwrap_or(i64::MAX))
            .bind(&case.input)
            .bind(expectation)
            .bind(&case.notes)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.get(&suite_id).await
    }

    pub async fn delete(&self, id: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM test_suites WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("test suite {id}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> (TestSuites, Db) {
        let db = Db::open_in_memory().await.unwrap();
        let now = now_ms();
        for id in ["srv", "other"] {
            sqlx::query("INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES (?, ?, 'stdio', 'x', ?, ?)")
                .bind(id).bind(id).bind(now).bind(now).execute(db.pool()).await.unwrap();
        }
        (TestSuites::new(db.clone()), db)
    }

    fn case(input: &str, expectation: Expectation) -> TestCaseInput {
        TestCaseInput {
            id: None,
            input: input.into(),
            expectation,
            notes: None,
        }
    }

    fn suite(name: &str) -> TestSuiteInput {
        TestSuiteInput {
            server_id: "srv".into(),
            name: name.into(),
            system_prompt: Some("  Be brief.  ".into()),
            cases: vec![
                case(
                    "What is open in a/b?",
                    Expectation::Tool {
                        name: "list_issues".into(),
                    },
                ),
                case("Say hello", Expectation::NoTool),
                case(
                    "Which repo?",
                    Expectation::Answer {
                        contains: "a/b".into(),
                    },
                ),
            ],
        }
    }

    #[tokio::test]
    async fn saves_a_suite_with_its_cases_in_order() {
        let (suites, _) = repo().await;
        let saved = suites.save(None, suite(" Smoke ")).await.unwrap();
        assert_eq!(saved.name, "Smoke");
        assert_eq!(saved.system_prompt.as_deref(), Some("Be brief."));
        assert_eq!(saved.cases.len(), 3);
        assert_eq!(
            saved.cases[0].expectation,
            Expectation::Tool {
                name: "list_issues".into()
            }
        );
        assert_eq!(saved.cases[1].expectation, Expectation::NoTool);
        assert_eq!(
            saved.cases[2].expectation,
            Expectation::Answer {
                contains: "a/b".into()
            }
        );
        assert_eq!(suites.get(&saved.id).await.unwrap(), saved);
    }

    #[tokio::test]
    async fn suites_are_saved_per_server_and_listed_by_name() {
        let (suites, _) = repo().await;
        suites.save(None, suite("beta")).await.unwrap();
        suites.save(None, suite("Alpha")).await.unwrap();
        let mut elsewhere = suite("gamma");
        elsewhere.server_id = "other".into();
        suites.save(None, elsewhere).await.unwrap();
        let names: Vec<_> = suites
            .list("srv")
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["Alpha", "beta"]);
        assert_eq!(suites.list("other").await.unwrap().len(), 1);
        assert!(suites.list("none").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn updating_replaces_the_cases_and_keeps_the_ids_that_were_sent_back() {
        let (suites, _) = repo().await;
        let saved = suites.save(None, suite("s")).await.unwrap();
        let kept = saved.cases[1].clone();
        let mut input = suite("renamed");
        input.system_prompt = None;
        input.cases = vec![
            TestCaseInput {
                id: Some(kept.id.clone()),
                input: "Say hi".into(),
                expectation: Expectation::NoTool,
                notes: Some(" note ".into()),
            },
            case("New", Expectation::Tool { name: "x".into() }),
        ];
        let updated = suites.save(Some(&saved.id), input).await.unwrap();
        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.system_prompt, None);
        assert_eq!(updated.cases.len(), 2);
        assert_eq!(updated.cases[0].id, kept.id);
        assert_eq!(updated.cases[0].notes.as_deref(), Some("note"));
        assert_ne!(updated.cases[1].id, kept.id);
        assert!(updated.updated_at >= saved.updated_at);
    }

    #[tokio::test]
    async fn rejects_invalid_suites() {
        let (suites, _) = repo().await;
        let mut nameless = suite("  ");
        assert!(suites.save(None, nameless.clone()).await.is_err());
        nameless.name = "ok".into();
        nameless.cases[0].input = "   ".into();
        assert!(suites
            .save(None, nameless)
            .await
            .unwrap_err()
            .to_string()
            .contains("case 1 has no input"));
        let mut no_tool = suite("s");
        no_tool.cases[0].expectation = Expectation::Tool { name: " ".into() };
        assert!(suites
            .save(None, no_tool)
            .await
            .unwrap_err()
            .to_string()
            .contains("names none"));
        let mut no_answer = suite("s");
        no_answer.cases[2].expectation = Expectation::Answer {
            contains: "".into(),
        };
        assert!(suites
            .save(None, no_answer)
            .await
            .unwrap_err()
            .to_string()
            .contains("case 3"));
        assert!(suites.list("srv").await.unwrap().is_empty());
        // A suite may have no cases yet.
        let mut empty = suite("empty");
        empty.cases.clear();
        assert!(suites.save(None, empty).await.unwrap().cases.is_empty());
    }

    #[tokio::test]
    async fn needs_an_existing_server_and_cannot_move_between_servers() {
        let (suites, _) = repo().await;
        let mut ghost = suite("s");
        ghost.server_id = "ghost".into();
        assert!(matches!(
            suites.save(None, ghost).await,
            Err(DbError::NotFound(_))
        ));
        let saved = suites.save(None, suite("s")).await.unwrap();
        let mut moved = suite("s");
        moved.server_id = "other".into();
        assert!(matches!(
            suites.save(Some(&saved.id), moved).await,
            Err(DbError::Invalid(_))
        ));
        assert!(matches!(
            suites.save(Some("missing"), suite("s")).await,
            Err(DbError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn deleting_a_suite_or_its_server_removes_the_cases() {
        let (suites, db) = repo().await;
        let a = suites.save(None, suite("a")).await.unwrap();
        suites.save(None, suite("b")).await.unwrap();
        suites.delete(&a.id).await.unwrap();
        assert!(matches!(suites.get(&a.id).await, Err(DbError::NotFound(_))));
        assert!(matches!(
            suites.delete(&a.id).await,
            Err(DbError::NotFound(_))
        ));
        assert_eq!(suites.list("srv").await.unwrap().len(), 1);
        sqlx::query("DELETE FROM servers WHERE id = 'srv'")
            .execute(db.pool())
            .await
            .unwrap();
        let cases: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_cases")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(cases, 0);
        assert!(suites.list("srv").await.unwrap().is_empty());
    }

    #[test]
    fn expectations_serialize_with_a_kind() {
        assert_eq!(
            serde_json::to_value(Expectation::NoTool).unwrap(),
            serde_json::json!({ "kind": "noTool" })
        );
        assert_eq!(
            serde_json::to_value(Expectation::Tool { name: "t".into() }).unwrap(),
            serde_json::json!({ "kind": "tool", "name": "t" })
        );
        assert_eq!(
            serde_json::from_value::<Expectation>(
                serde_json::json!({ "kind": "answer", "contains": "x" })
            )
            .unwrap(),
            Expectation::Answer {
                contains: "x".into()
            }
        );
    }
}
