//! Desktop effect operations.

use ora_contracts::*;

backend_command!(
    get_effect_target_status,
    GetEffectTargetStatusRequest,
    GetEffectTargetStatusResponse,
    effects.target_status,
    "Loads one generic Effect Target status."
);

backend_command!(
    recover_effect_target,
    RecoverEffectTargetRequest,
    RecoverEffectTargetResponse,
    effects.recover_target,
    "Closes one quarantined Effect Target's journals by exact adapter evidence."
);
