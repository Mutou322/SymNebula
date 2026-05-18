use std::collections::HashMap;
use slotmap::SlotMap;

use crate::ids::SymbolId;

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct SymbolTable {
    map: HashMap<String, SymbolId>,
    symbols: SlotMap<SymbolId, Symbol>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            symbols: SlotMap::with_key(),
        }
    }

    pub fn intern(&mut self, s: &str) -> SymbolId {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let id = self.symbols.insert(Symbol {
            name: s.to_string(),
        });
        self.map.insert(s.to_string(), id);
        id
    }

    pub fn get(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id)
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn contains(&self, s: &str) -> bool {
        self.map.contains_key(s)
    }
}
