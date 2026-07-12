//! Parameter-grid expansion: a spec template with `"$name"` placeholders plus
//! per-parameter value lists → one named variant per combination.

/// A grid file: a spec template plus parameter value lists. Inside `spec`, any
/// JSON string equal to `"$name"` is a placeholder for the parameter `name`.
#[derive(serde::Deserialize)]
pub struct GridSpec {
    pub spec: serde_json::Value,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, Vec<serde_json::Value>>,
}

fn substitute(
    node: &serde_json::Value,
    binding: &std::collections::BTreeMap<&str, &serde_json::Value>,
) -> serde_json::Value {
    match node {
        serde_json::Value::String(s) => {
            if let Some(name) = s.strip_prefix('$') {
                if let Some(v) = binding.get(name) {
                    return (*v).clone();
                }
            }
            node.clone()
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute(v, binding)))
                .collect(),
        ),
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| substitute(v, binding)).collect())
        }
        _ => node.clone(),
    }
}

/// Expand a [`GridSpec`] into one named variant per parameter combination
/// (cartesian product, parameter order = alphabetical by name). Names look
/// like `"n=10,thresh=0.5"`. A grid with no params yields the spec itself.
pub fn expand_grid(grid: &GridSpec) -> Vec<(String, serde_json::Value)> {
    let names: Vec<&String> = grid.params.keys().collect();
    let lists: Vec<&Vec<serde_json::Value>> = grid.params.values().collect();
    if names.is_empty() {
        return vec![("base".to_string(), grid.spec.clone())];
    }
    let mut out = Vec::new();
    let mut idx = vec![0usize; names.len()];
    loop {
        // names[k] and lists[k] come from the same BTreeMap iteration order.
        let binding: std::collections::BTreeMap<&str, &serde_json::Value> = (0..names.len())
            .map(|k| (names[k].as_str(), &lists[k][idx[k]]))
            .collect();
        let name = (0..names.len())
            .map(|k| format!("{}={}", names[k], lists[k][idx[k]]))
            .collect::<Vec<_>>()
            .join(",");
        out.push((name, substitute(&grid.spec, &binding)));
        // odometer increment
        let mut k = names.len();
        loop {
            if k == 0 {
                return out;
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < lists[k].len() {
                break;
            }
            idx[k] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_params_yields_the_base_spec() {
        let g = GridSpec {
            spec: json!({"op": "x"}),
            params: Default::default(),
        };
        let out = expand_grid(&g);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "base");
        assert_eq!(out[0].1, json!({"op": "x"}));
    }

    #[test]
    fn cartesian_product_names_and_substitutes() {
        let mut params = std::collections::BTreeMap::new();
        params.insert("n".to_string(), vec![json!(10), json!(20)]);
        params.insert("t".to_string(), vec![json!(0.5)]);
        let g = GridSpec {
            spec: json!({"n": "$n", "t": "$t", "keep": "$unbound"}),
            params,
        };
        let out = expand_grid(&g);
        // 2 × 1 combinations, parameter order alphabetical (n, then t).
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "n=10,t=0.5");
        // `$n`/`$t` substituted; an unbound `$unbound` is left as-is.
        assert_eq!(out[0].1, json!({"n": 10, "t": 0.5, "keep": "$unbound"}));
        assert_eq!(out[1].0, "n=20,t=0.5");
        assert_eq!(out[1].1, json!({"n": 20, "t": 0.5, "keep": "$unbound"}));
    }
}
