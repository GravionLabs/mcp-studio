//! The editable price table: what a million tokens cost per model.
//!
//! No prices are built in. Providers change them often, so the user enters the models and prices
//! they care about and every cost shown in the app is computed from this table.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::FromRow;

use crate::db::{Db, DbError, DbResult};

/// Price of one model, in `currency` per million tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: Option<f64>,
    pub cache_write_per_mtok: Option<f64>,
    /// Currency code such as `USD`.
    pub currency: String,
}

impl Price {
    /// Trims text, upper-cases the currency, and rejects negative or non-finite amounts.
    pub fn normalized(mut self) -> DbResult<Self> {
        self.model = self.model.trim().to_owned();
        if self.model.is_empty() {
            return Err(DbError::Invalid("model is required".into()));
        }
        self.currency = self.currency.trim().to_uppercase();
        if self.currency.is_empty() {
            self.currency = "USD".into();
        }
        if self.currency.len() != 3 || !self.currency.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(DbError::Invalid(
                "currency must be a three-letter code such as USD".into(),
            ));
        }
        for (label, value) in [
            ("input price", Some(self.input_per_mtok)),
            ("output price", Some(self.output_per_mtok)),
            ("cache read price", self.cache_read_per_mtok),
            ("cache write price", self.cache_write_per_mtok),
        ] {
            if value.is_some_and(|v| !v.is_finite() || v < 0.0) {
                return Err(DbError::Invalid(format!(
                    "{label} must be a number of zero or more"
                )));
            }
        }
        Ok(self)
    }

    /// Cost of tokens a model reads (prompts, tool definitions, tool results).
    pub fn input_cost(&self, tokens: u64) -> f64 {
        tokens as f64 * self.input_per_mtok / 1_000_000.0
    }

    /// Cost of tokens a model writes (tool call arguments).
    pub fn output_cost(&self, tokens: u64) -> f64 {
        tokens as f64 * self.output_per_mtok / 1_000_000.0
    }
}

/// An amount of money.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Clone, Debug)]
pub struct Prices {
    db: Db,
}

impl Prices {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<Price>> {
        Ok(sqlx::query_as(
            "SELECT model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok, currency \
             FROM prices ORDER BY model COLLATE NOCASE",
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    pub async fn get(&self, model: &str) -> DbResult<Price> {
        let row: Option<Price> = sqlx::query_as(
            "SELECT model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok, currency \
             FROM prices WHERE model = ?",
        )
        .bind(model)
        .fetch_optional(self.db.pool())
        .await?;
        row.ok_or_else(|| DbError::NotFound(format!("price for model {model}")))
    }

    /// Adds a price or replaces the one of the same model.
    pub async fn set(&self, price: Price) -> DbResult<Price> {
        let price = price.normalized()?;
        sqlx::query(
            "INSERT INTO prices (model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok, currency) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (model) DO UPDATE SET input_per_mtok = excluded.input_per_mtok, \
               output_per_mtok = excluded.output_per_mtok, cache_read_per_mtok = excluded.cache_read_per_mtok, \
               cache_write_per_mtok = excluded.cache_write_per_mtok, currency = excluded.currency",
        )
        .bind(&price.model)
        .bind(price.input_per_mtok)
        .bind(price.output_per_mtok)
        .bind(price.cache_read_per_mtok)
        .bind(price.cache_write_per_mtok)
        .bind(&price.currency)
        .execute(self.db.pool())
        .await?;
        self.get(&price.model).await
    }

    pub async fn delete(&self, model: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM prices WHERE model = ?")
            .bind(model)
            .execute(self.db.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("price for model {model}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(model: &str) -> Price {
        Price {
            model: model.into(),
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: None,
            cache_write_per_mtok: None,
            currency: "usd".into(),
        }
    }

    async fn repo() -> Prices {
        Prices::new(Db::open_in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn starts_empty_and_has_no_built_in_prices() {
        assert!(repo().await.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn adds_prices_and_normalizes_them() {
        let prices = repo().await;
        let saved = prices.set(price("  model-a ")).await.unwrap();
        assert_eq!(saved.model, "model-a");
        assert_eq!(saved.currency, "USD");
        assert_eq!(prices.list().await.unwrap(), vec![saved]);
    }

    #[tokio::test]
    async fn setting_an_existing_model_replaces_its_price() {
        let prices = repo().await;
        prices.set(price("m")).await.unwrap();
        let mut changed = price("m");
        changed.input_per_mtok = 1.5;
        changed.cache_read_per_mtok = Some(0.3);
        prices.set(changed.clone()).await.unwrap();
        let all = prices.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].input_per_mtok, 1.5);
        assert_eq!(all[0].cache_read_per_mtok, Some(0.3));
    }

    #[tokio::test]
    async fn lists_models_alphabetically_ignoring_case() {
        let prices = repo().await;
        for model in ["b", "A", "c"] {
            prices.set(price(model)).await.unwrap();
        }
        let models: Vec<_> = prices
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.model)
            .collect();
        assert_eq!(models, ["A", "b", "c"]);
    }

    #[tokio::test]
    async fn rejects_invalid_prices() {
        let prices = repo().await;
        assert!(prices.set(price("  ")).await.is_err());
        let mut negative = price("m");
        negative.output_per_mtok = -1.0;
        assert!(prices.set(negative).await.is_err());
        let mut nan = price("m");
        nan.input_per_mtok = f64::NAN;
        assert!(prices.set(nan).await.is_err());
        let mut bad_currency = price("m");
        bad_currency.currency = "dollars".into();
        assert!(prices.set(bad_currency).await.is_err());
        assert!(prices.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deletes_prices() {
        let prices = repo().await;
        prices.set(price("m")).await.unwrap();
        prices.delete("m").await.unwrap();
        assert!(prices.list().await.unwrap().is_empty());
        assert!(matches!(
            prices.delete("m").await,
            Err(DbError::NotFound(_))
        ));
    }

    #[test]
    fn computes_costs_per_million_tokens() {
        let p = price("m");
        assert!((p.input_cost(1_000_000) - 3.0).abs() < 1e-9);
        assert!((p.output_cost(2_000) - 0.03).abs() < 1e-9);
        assert_eq!(p.input_cost(0), 0.0);
    }
}
