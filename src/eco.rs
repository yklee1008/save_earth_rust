use crate::power;

const CARBON_COEFFICIENT_PER_KWH: f64 = 0.456;
const PINE_TREE_ABSORPTION_YEARLY: f64 = 6.6;

#[derive(Debug, Clone)]
pub struct EcoMetrics {
    pub today_wh: f64,
    pub month_wh: f64,
    pub saved_wh: f64,
    pub carbon_saved_kg: f64,
    pub pine_trees: f64,
}

/// power.rs에서 누적 전력량을 가져와 친환경 지표를 계산합니다.
pub fn get_eco_metrics() -> EcoMetrics {
    // 1. power.rs에서 일간/월간 전체 전력량 [kWh] 가져오기
    let today_kwh = power::get_daily_energy_kwh();
    let month_kwh = power::get_monthly_energy_kwh();

    // 2. kWh -> Wh 단위 변환
    let today_wh = today_kwh * 1000.0;
    let month_wh = month_kwh * 1000.0;

    // 3. 누적 저장된 절감 전력량(kWh)을 가져와서 Wh로 변환
    // (power.rs에서 균형 모드 누적 시 이미 12%를 계산하여 monthly_saved_energy_kwh에 저장 중임)
    let saved_kwh = power::get_saved_energy_kwh();
    let saved_wh = saved_kwh * 1000.0;
    
    // 4. 탄소 절감량 및 소나무 식재 효과 계산 (kWh 기준)
    let carbon_saved = saved_kwh * CARBON_COEFFICIENT_PER_KWH;
    let pine_trees = carbon_saved / PINE_TREE_ABSORPTION_YEARLY;

    EcoMetrics {
        today_wh,
        month_wh,
        saved_wh,
        carbon_saved_kg: carbon_saved,
        pine_trees,
    }
}