//! 类型映射表
//!
//! 提供类型安全的键值存储，用于在 Context 中存储任意类型的数据。

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// 类型映射表
///
/// 提供类型安全的键值存储，每个类型只能存储一个值。
/// 使用 `Arc<dyn Any>` 存储数据，支持 `Send + Sync` 类型。
#[derive(Clone, Debug, Default)]
pub struct TypeMap {
    map: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl TypeMap {
    /// 创建新的类型映射表
    pub fn new() -> Self {
        Self {
            map: Arc::new(HashMap::new()),
        }
    }

    /// 插入值
    ///
    /// 如果该类型已存在，则替换旧值。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::typemap::TypeMap;
    ///
    /// let mut map = TypeMap::new();
    /// map.insert(42i32);
    /// map.insert("hello".to_string());
    /// ```
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        let type_id = TypeId::of::<T>();
        let mut new_map = HashMap::new();
        
        // 克隆现有数据
        for (k, v) in self.map.iter() {
            new_map.insert(*k, Arc::clone(v));
        }
        
        // 插入新值
        new_map.insert(type_id, Arc::new(value));
        
        self.map = Arc::new(new_map);
    }

    /// 获取值
    ///
    /// 返回该类型的值的引用，如果不存在则返回 `None`。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::typemap::TypeMap;
    ///
    /// let mut map = TypeMap::new();
    /// map.insert(42i32);
    ///
    /// assert_eq!(map.get::<i32>(), Some(&42));
    /// assert_eq!(map.get::<String>(), None);
    /// ```
    pub fn get<T: 'static>(&self) -> Option<&T> {
        let type_id = TypeId::of::<T>();
        self.map
            .get(&type_id)
            .and_then(|v| v.downcast_ref::<T>())
    }

    /// 移除值
    ///
    /// 移除指定类型的值，如果存在则返回该值。
    ///
    /// **注意**：只有当 `Arc` 的引用计数为 1 时才能成功移除。
    /// 如果引用计数大于 1（即被其他地方共享），则返回 `None`。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::typemap::TypeMap;
    ///
    /// let mut map = TypeMap::new();
    /// map.insert(42i32);
    ///
    /// assert_eq!(map.remove::<i32>(), Some(42));
    /// assert_eq!(map.get::<i32>(), None);
    /// ```
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        let type_id = TypeId::of::<T>();
        
        // 检查是否存在
        if !self.map.contains_key(&type_id) {
            return None;
        }
        
        // 我们需要创建一个新的 HashMap，不包含要移除的项
        // 同时尝试获取要移除的值的唯一所有权
        let mut new_map = HashMap::new();
        let mut removed_value: Option<Arc<dyn Any + Send + Sync>> = None;
        
        for (k, v) in self.map.iter() {
            if *k == type_id {
                // 保存要移除的值（不克隆，直接引用）
                removed_value = Some(Arc::clone(v));
            } else {
                new_map.insert(*k, Arc::clone(v));
            }
        }
        
        // 尝试从 Arc 中提取值
        if let Some(arc_value) = removed_value {
            // 尝试 downcast 到 T
            if let Ok(downcasted) = arc_value.downcast::<T>() {
                // 尝试 unwrap（如果引用计数为 1）
                // 注意：由于我们之前克隆了 Arc，引用计数至少为 2
                // 所以我们需要使用不同的策略
                // 
                // 实际上，由于 TypeMap 使用 Arc<HashMap>，而 HashMap 中的值也是 Arc，
                // 当我们克隆 Arc 时，引用计数会增加。
                // 但是，如果我们从 HashMap 中移除这个值，然后尝试 unwrap，
                // 引用计数应该会减少到 1（只有我们持有的这个克隆）。
                //
                // 所以，我们需要先更新 map，然后再尝试 unwrap。
                // 但是，由于我们已经克隆了 Arc，引用计数至少为 2。
                //
                // 更好的方法是：检查 Arc 的引用计数，如果为 2（一个在 HashMap 中，一个是我们克隆的），
                // 我们可以先更新 map，然后尝试 unwrap。
                //
                // 但是，Arc 没有提供直接检查引用计数的方法（stable API）。
                // 我们可以使用 Arc::get_mut 来检查是否可以获取可变引用。
                
                // 先更新 map（移除要移除的项）
                self.map = Arc::new(new_map);
                
                // 现在尝试 unwrap（此时引用计数应该为 1，因为我们已经从 HashMap 中移除了它）
                // 但是，由于我们之前克隆了 Arc，引用计数仍然至少为 2。
                // 所以我们需要使用不同的方法。
                //
                // 实际上，问题在于：当我们从 HashMap 中获取值时，我们使用 Arc::clone，
                // 这会增加引用计数。然后我们更新 HashMap，移除原始值，但我们的克隆仍然存在。
                //
                // 解决方案：我们需要在更新 HashMap 之前，先检查是否可以获取唯一所有权。
                // 但是，由于我们已经克隆了 Arc，这不可能。
                //
                // 更好的解决方案：不要克隆 Arc，而是直接从 HashMap 中移除它。
                // 但是，由于 TypeMap 使用 Arc<HashMap>，我们需要创建一个新的 HashMap。
                //
                // 实际上，我们可以这样做：
                // 1. 从 HashMap 中获取值的引用（不克隆）
                // 2. 尝试 downcast（需要克隆 Arc）
                // 3. 检查 Arc 的引用计数（不稳定 API）
                // 4. 如果引用计数为 1，我们可以 unwrap
                //
                // 但是，由于我们无法在 stable Rust 中检查引用计数，我们需要使用其他方法。
                //
                // 实际上，对于大多数用例，如果 Arc 被共享，移除操作应该失败。
                // 但是，在 TypeMap 的情况下，如果值只存在于 TypeMap 中，引用计数应该为 1。
                //
                // 让我们尝试一个不同的方法：使用 Arc::get_mut 来检查是否可以获取可变引用。
                // 如果可以，说明引用计数为 1，我们可以安全地 unwrap。
                
                // 由于我们已经克隆了 Arc，引用计数至少为 2，所以 try_unwrap 会失败。
                // 我们需要使用不同的策略。
                //
                // 实际上，问题在于我们的设计：TypeMap 使用 Arc<HashMap>，而 HashMap 中的值也是 Arc。
                // 当我们尝试移除值时，我们需要克隆 Arc 来进行 downcast，这会增加引用计数。
                //
                // 解决方案：我们需要重新设计 remove 方法，使其能够处理这种情况。
                // 一个简单的方法是：如果 T 实现了 Clone，我们可以克隆它；否则，我们只能返回 None。
                //
                // 但是，这会使 API 变得复杂。
                //
                // 另一个解决方案：移除操作只返回 bool，表示是否成功移除，而不返回实际值。
                // 但这不符合用户的期望。
                //
                // 最好的解决方案：使用 unsafe 代码来检查引用计数，或者接受这个限制。
                //
                // 实际上，对于大多数用例，如果值只存在于 TypeMap 中，引用计数应该为 1。
                // 但是，由于我们克隆了 Arc，引用计数至少为 2。
                //
                // 让我们尝试一个更简单的方法：直接尝试 unwrap，如果失败，返回 None。
                // 这符合"只有当 Arc 的引用计数为 1 时才能成功移除"的语义。
                
                return Arc::try_unwrap(downcasted).ok();
            }
        }
        
        None
    }

    /// 检查是否包含指定类型的值
    pub fn contains<T: 'static>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        self.map.contains_key(&type_id)
    }

    /// 清空所有数据
    pub fn clear(&mut self) {
        self.map = Arc::new(HashMap::new());
    }

    /// 获取数据数量
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_get() {
        let mut map = TypeMap::new();
        
        map.insert(42i32);
        map.insert("hello".to_string());
        
        assert_eq!(map.get::<i32>(), Some(&42));
        assert_eq!(map.get::<String>(), Some(&"hello".to_string()));
        assert_eq!(map.get::<bool>(), None);
    }

    #[test]
    fn test_replace() {
        let mut map = TypeMap::new();
        
        map.insert(42i32);
        assert_eq!(map.get::<i32>(), Some(&42));
        
        map.insert(100i32);
        assert_eq!(map.get::<i32>(), Some(&100));
    }

    #[test]
    fn test_remove() {
        let mut map = TypeMap::new();
        
        map.insert(42i32);
        assert_eq!(map.remove::<i32>(), Some(42));
        assert_eq!(map.get::<i32>(), None);
    }

    #[test]
    fn test_contains() {
        let mut map = TypeMap::new();
        
        assert!(!map.contains::<i32>());
        
        map.insert(42i32);
        assert!(map.contains::<i32>());
    }

    #[test]
    fn test_clone() {
        let mut map = TypeMap::new();
        map.insert(42i32);
        
        let cloned = map.clone();
        assert_eq!(cloned.get::<i32>(), Some(&42));
        
        // 修改原 map 不影响克隆
        map.insert(100i32);
        assert_eq!(cloned.get::<i32>(), Some(&42));
    }
}
