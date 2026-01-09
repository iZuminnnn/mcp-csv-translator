use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct TerminologyCache {
    terms: Arc<RwLock<HashMap<String, String>>>,
    max_size: usize,
}

impl TerminologyCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            terms: Arc::new(RwLock::new(HashMap::new())),
            max_size,
        }
    }

    pub async fn get(&self, source: &str) -> Option<String> {
        let terms = self.terms.read().await;
        terms.get(source).cloned()
    }

    pub async fn insert(&self, source: String, target: String) {
        let mut terms = self.terms.write().await;

        if terms.len() >= self.max_size {
            if let Some(key) = terms.keys().next().cloned() {
                terms.remove(&key);
            }
        }

        terms.insert(source, target);
    }

    pub async fn insert_batch(&self, new_terms: HashMap<String, String>) {
        let mut terms = self.terms.write().await;

        let available_space = self.max_size.saturating_sub(terms.len());
        let to_insert: Vec<_> = new_terms.into_iter().take(available_space).collect();

        for (source, target) in to_insert {
            terms.insert(source, target);
        }
    }

    pub async fn get_all(&self) -> HashMap<String, String> {
        self.terms.read().await.clone()
    }

    pub async fn len(&self) -> usize {
        self.terms.read().await.len()
    }

    pub async fn clear(&self) {
        self.terms.write().await.clear();
    }

    pub async fn remove(&self, source: &str) -> Option<String> {
        self.terms.write().await.remove(source)
    }
}

impl Default for TerminologyCache {
    fn default() -> Self {
        Self::new(10000)
    }
}
