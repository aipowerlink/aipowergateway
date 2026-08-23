//! 配置工具：schema 驱动（类型/默认/角色/敏感度声明）。
//! 0.1.0 基础版：合并 + 敏感值标记；完整 schema 校验后续补。

use serde_json::Value;

/// 敏感度（对应 DSH settings redact 的 role('secret')）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    /// 普通配置。
    Normal,
    /// 敏感值（密码/token）：读取/导出/日志脱敏。
    Secret,
}

/// 配置项声明：schema 驱动的基础单元。
#[derive(Debug, Clone)]
pub struct ConfigField {
    /// 配置键（点分路径）。
    pub key: &'static str,
    /// 默认值。
    pub default: Value,
    /// 敏感度。
    pub sensitivity: Sensitivity,
    /// 角色归属：server / client / global。
    pub role: &'static str,
}

impl ConfigField {
    pub const fn new(key: &'static str, default: Value, role: &'static str) -> Self {
        Self { key, default, sensitivity: Sensitivity::Normal, role }
    }

    pub const fn secret(mut self) -> Self {
        self.sensitivity = Sensitivity::Secret;
        self
    }
}

/// 合并配置：字段声明 → 用户覆盖（深层合并）。
pub fn merge(fields: &[ConfigField], base: &Value, overrides: &Value) -> Value {
    let mut out = base.clone();
    for f in fields {
        if out.get(f.key).is_none() {
            set_path(&mut out, f.key, f.default.clone());
        }
    }
    deep_merge(&mut out, overrides);
    out
}

/// 深度合并 override 到 target。
pub fn deep_merge(target: &mut Value, overrides: &Value) {
    if let (Value::Object(t), Value::Object(o)) = (target, overrides) {
        for (k, v) in o {
            if let Some(existing) = t.get_mut(k) {
                if existing.is_object() && v.is_object() {
                    deep_merge(existing, v);
                } else {
                    t.insert(k.clone(), v.clone());
                }
            } else {
                t.insert(k.clone(), v.clone());
            }
        }
    }
}

/// 按点分路径设置值（仅在最终路径写入；中间路径不存在则跳过）。
fn set_path(target: &mut Value, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.len() == 1 {
        if let Value::Object(map) = target {
            map.insert(parts[0].to_string(), value);
        }
        return;
    }
    // 逐层下降：全部中间段必须是已存在的 object，否则跳过
    let mut current = target;
    for part in &parts[..parts.len() - 1] {
        match current.get_mut(*part) {
            Some(v @ Value::Object(_)) => current = v,
            _ => return,
        }
    }
    if let Some(map) = current.as_object_mut() {
        map.insert(parts[parts.len() - 1].to_string(), value);
    }
}

/// 脱敏：secret 字段替换为占位（对应 DSH redact）。
pub fn redact(fields: &[ConfigField], value: &Value) -> Value {
    let mut out = value.clone();
    for f in fields {
        if f.sensitivity == Sensitivity::Secret {
            if let Some(v) = out.get_mut(f.key) {
                *v = if v.is_null() { Value::Null } else { Value::Bool(true) };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_applies_defaults_and_overrides() {
        let fields = [
            ConfigField::new("port", json!(39091), "server"),
            ConfigField::new("password", json!(null), "server").secret(),
        ];
        let merged = merge(&fields, &json!({}), &json!({ "port": 40000 }));
        assert_eq!(merged["port"], 40000);
        assert!(merged.get("password").is_some());
    }

    #[test]
    fn redact_hides_secret() {
        let fields = [ConfigField::new("password", json!(null), "server").secret()];
        let v = json!({ "password": "hunter2", "port": 39091 });
        let r = redact(&fields, &v);
        assert_eq!(r["password"], json!(true));
        assert_eq!(r["port"], 39091);
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn deep_merge_nested() {
        let mut t = json!({ "a": { "b": 1 } });
        deep_merge(&mut t, &json!({ "a": { "c": 2 } }));
        assert_eq!(t["a"]["b"], 1);
        assert_eq!(t["a"]["c"], 2);
    }
}