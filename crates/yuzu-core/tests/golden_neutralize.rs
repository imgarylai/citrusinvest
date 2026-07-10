mod golden_harness;
use golden_harness::{assert_panel_eq, load_golden, panel_from_json};
use yuzu_core::panel::Panel;

fn input(name: &str) -> Panel {
    panel_from_json(&load_golden(name), "input")
}
fn expected(name: &str) -> Panel {
    panel_from_json(&load_golden(name), "expected")
}

#[test]
fn neutralize_matches_reference() {
    let v = load_golden("neutralize");
    let factor = panel_from_json(&v, "input");
    let size = panel_from_json(&v, "size");
    let got = factor.neutralize(&[size], true);
    assert_panel_eq(&got, &expected("neutralize"), 1e-9);
}

#[test]
fn neutralize_industry_matches_reference() {
    let v = load_golden("neutralize_industry");
    let factor = panel_from_json(&v, "input");
    let industry: std::collections::HashMap<String, String> = factor
        .symbols
        .iter()
        .cloned()
        .zip(
            v["industry"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string()),
        )
        .collect();
    let got = factor.neutralize_industry(&industry, true);
    assert_panel_eq(&got, &expected("neutralize_industry"), 1e-9);
}

#[test]
fn industry_rank_matches_reference() {
    let v = load_golden("industry_rank");
    let factor = panel_from_json(&v, "input");
    let industry: std::collections::HashMap<String, String> = factor
        .symbols
        .iter()
        .cloned()
        .zip(
            v["industry"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string()),
        )
        .collect();
    let got = factor.industry_rank(&industry, None);
    assert_panel_eq(&got, &expected("industry_rank"), 1e-9);
}

#[test]
fn groupby_category_matches_reference() {
    let v = load_golden("groupby_category");
    let factor = panel_from_json(&v, "input");
    let industry: std::collections::HashMap<String, String> = factor
        .symbols
        .iter()
        .cloned()
        .zip(
            v["industry"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string()),
        )
        .collect();
    for agg in ["mean", "sum", "std"] {
        let got = factor.groupby_category(&industry, agg).unwrap();
        let exp = panel_from_json(&v, agg); // dates x [Energy, Tech] via expected_* axes
        assert_panel_eq(&got, &exp, 1e-9);
    }
}

#[test]
fn cap_industry_matches_reference() {
    let v = load_golden("cap_industry");
    let weights = panel_from_json(&v, "input");
    let industry: std::collections::HashMap<String, String> = weights
        .symbols
        .iter()
        .cloned()
        .zip(
            v["industry"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string()),
        )
        .collect();
    let max_weight = v["max_weight"].as_f64().unwrap();
    let got = weights.cap_industry(&industry, max_weight);
    assert_panel_eq(&got, &expected("cap_industry"), 1e-9);
}

#[test]
fn groupby_category_rejects_invalid_agg() {
    let v = load_golden("groupby_category");
    let factor = panel_from_json(&v, "input");
    let industry: std::collections::HashMap<String, String> = factor
        .symbols
        .iter()
        .cloned()
        .zip(
            v["industry"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string()),
        )
        .collect();
    assert!(factor.groupby_category(&industry, "invalid").is_err());
}
