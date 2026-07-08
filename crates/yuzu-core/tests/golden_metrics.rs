mod golden_harness;
use yuzu_core::metrics;
use golden_harness::load_golden;

fn eq_dates(name: &str) -> (Vec<f64>, Vec<i32>) {
    let v = load_golden(name);
    let equity = v["equity"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    let dates = v["dates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap().replace('-', "").parse().unwrap())
        .collect();
    (equity, dates)
}

fn want(name: &str, key: &str) -> f64 {
    load_golden(name)["metrics"][key].as_f64().unwrap()
}

#[test]
fn metrics_match_ffn() {
    let (eq, dates) = eq_dates("metrics_basic");
    let tol = 1e-9;
    assert!((metrics::total_return(&eq) - want("metrics_basic", "total_return")).abs() < tol);
    assert!((metrics::cagr(&eq, &dates) - want("metrics_basic", "cagr")).abs() < 1e-6);
    assert!((metrics::max_drawdown(&eq) - want("metrics_basic", "max_drawdown")).abs() < tol);
    assert!((metrics::ann_volatility(&eq) - want("metrics_basic", "ann_volatility")).abs() < 1e-9);
    assert!((metrics::sharpe(&eq) - want("metrics_basic", "sharpe")).abs() < 1e-9);
    assert!((metrics::sortino(&eq) - want("metrics_basic", "sortino")).abs() < 1e-9);
    assert!((metrics::calmar(&eq, &dates) - want("metrics_basic", "calmar")).abs() < 1e-6);
}
