// Execution state: distributed state tracking

use std::collections::HashMap;

/// ExecutionState: distributed state snapshot
#[derive(Debug, Clone)]
pub struct ExecutionState {
    state_map: HashMap<String, String>,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            state_map: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: String, value: String) {
        self.state_map.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.state_map.get(key).cloned()
    }

    pub fn snapshot(&self) -> HashMap<String, String> {
        self.state_map.clone()
    }
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let state = ExecutionState::new();
        assert!(state.get("key").is_none());
    }

    #[test]
    fn test_set_and_get() {
        let mut state = ExecutionState::new();
        state.set("key1".to_string(), "value1".to_string());

        assert_eq!(state.get("key1"), Some("value1".to_string()));
    }
}
