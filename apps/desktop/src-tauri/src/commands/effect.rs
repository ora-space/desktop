//! Desktop effect operations.

use ora_contracts::*;

backend_command!(
    get_effect_target_status,
    GetEffectTargetStatusRequest,
    GetEffectTargetStatusResponse,
    effects.target_status,
    "Loads one generic Effect Target status."
);
