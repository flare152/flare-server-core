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
        self.map.get(&type_id).and_then(|v| v.downcast_ref::<T>())
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
                // 先更新 map（移除要移除的项）
                self.map = Arc::new(new_map);

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
