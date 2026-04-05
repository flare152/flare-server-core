//! 拓扑排序实现
//!
//! 用于确定任务的启动顺序

use std::collections::{HashMap, HashSet, VecDeque};

/// 拓扑排序
///
/// 使用 Kahn 算法进行拓扑排序，确保依赖的任务先启动
/// 同时检测循环依赖
///
/// # 参数
///
/// * `items` - 待排序的项列表，每项包含名称和依赖列表
///
/// # 返回
///
/// - `Ok(sorted)` - 排序后的项列表
/// - `Err(cycle)` - 检测到循环依赖，返回涉及的任务
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::utils::topological_sort;
///
/// let items = vec![
///     ("task-a", vec![]),
///     ("task-b", vec!["task-a".to_string()]),
///     ("task-c", vec!["task-a".to_string()]),
/// ];
///
/// let sorted = topological_sort(items).unwrap();
/// assert_eq!(sorted[0], "task-a");
/// ```
pub fn topological_sort(items: Vec<(String, Vec<String>)>) -> Result<Vec<String>, Vec<String>> {
    // 构建任务索引
    let mut item_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut item_names = Vec::new();

    for (name, deps) in items {
        item_names.push(name.clone());
        item_map.insert(name, deps);
    }

    // 构建依赖图：item -> 依赖它的项列表
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    for name in &item_names {
        dependents.insert(name.clone(), Vec::new());
        in_degree.insert(name.clone(), 0);
    }

    // 构建依赖关系
    for name in &item_names {
        let deps = item_map.get(name).unwrap();

        for dep in deps {
            // 检查依赖的项是否存在
            if !item_map.contains_key(dep) {
                return Err(vec![format!(
                    "Item '{}' depends on '{}', but '{}' is not registered",
                    name, dep, dep
                )]);
            }

            // 增加入度
            *in_degree.get_mut(name).unwrap() += 1;

            // 添加到依赖者的列表
            dependents.get_mut(dep).unwrap().push(name.clone());
        }
    }

    // Kahn 算法：找到所有入度为 0 的项
    let mut queue = VecDeque::new();
    for (name, degree) in &in_degree {
        if *degree == 0 {
            queue.push_back(name.clone());
        }
    }

    let mut sorted = Vec::new();
    let mut processed = HashSet::new();

    // 处理队列中的项
    while let Some(current) = queue.pop_front() {
        if processed.contains(&current) {
            continue;
        }

        processed.insert(current.clone());

        // 将项添加到排序结果
        sorted.push(current.clone());

        // 更新依赖此项的其他项的入度
        if let Some(deps) = dependents.get(&current) {
            for dependent in deps {
                let degree = in_degree.get_mut(dependent).unwrap();
                *degree -= 1;

                if *degree == 0 {
                    queue.push_back(dependent.clone());
                }
            }
        }
    }

    // 检查是否有循环依赖
    if sorted.len() != item_names.len() {
        let remaining: Vec<String> = item_names
            .into_iter()
            .filter(|name| !processed.contains(name))
            .collect();

        return Err(remaining);
    }

    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topological_sort_no_deps() {
        let items = vec![
            ("task-a".to_string(), vec![]),
            ("task-b".to_string(), vec![]),
            ("task-c".to_string(), vec![]),
        ];

        let sorted = topological_sort(items).unwrap();
        assert_eq!(sorted.len(), 3);
    }

    #[test]
    fn test_topological_sort_with_deps() {
        let items = vec![
            ("task-a".to_string(), vec![]),
            ("task-b".to_string(), vec!["task-a".to_string()]),
            ("task-c".to_string(), vec!["task-b".to_string()]),
        ];

        let sorted = topological_sort(items).unwrap();
        assert_eq!(sorted, vec!["task-a", "task-b", "task-c"]);
    }

    #[test]
    fn test_topological_sort_multiple_deps() {
        let items = vec![
            ("task-a".to_string(), vec![]),
            ("task-b".to_string(), vec![]),
            (
                "task-c".to_string(),
                vec!["task-a".to_string(), "task-b".to_string()],
            ),
        ];

        let sorted = topological_sort(items).unwrap();
        // task-a 和 task-b 的顺序不确定，但 task-c 一定在最后
        assert_eq!(sorted[2], "task-c");
    }

    #[test]
    fn test_topological_sort_cycle() {
        let items = vec![
            ("task-a".to_string(), vec!["task-b".to_string()]),
            ("task-b".to_string(), vec!["task-a".to_string()]),
        ];

        let result = topological_sort(items);
        assert!(result.is_err());
    }

    #[test]
    fn test_topological_sort_missing_dep() {
        let items = vec![("task-a".to_string(), vec!["task-b".to_string()])];

        let result = topological_sort(items);
        assert!(result.is_err());
    }
}
