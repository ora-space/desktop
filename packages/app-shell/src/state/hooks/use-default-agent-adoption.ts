import { useEffect } from "react";
import { useSettingsStore } from "../stores/settings-store";
import { useAgentRuntimeStatus } from "./use-agent-runtime-status";
import { useAvailableAgents } from "./use-available-agents";

/**
 * Gives an installation that has never chosen an agent one to start on.
 *
 * `settings.agentCli` begins unset, and every chat surface nobody has pointed anywhere resolves
 * through it, so a first run opens on no agent at all: the picker names none, the send gate
 * blocks, and starting a session reports that one must be picked — even when a single agent is
 * installed and reachable and there is nothing to choose between. Adopting the first agent the
 * runtime reports reaching is what makes that first message sendable.
 *
 * The preference is written rather than resolved as a fallback inside `useTargetAgentCli`, and
 * that distinction is the whole design. A fallback would be re-derived on every render, so a
 * package being installed, disabled, or restarted would move a user who never asked to move.
 * Adoption happens once: afterwards `agentCli` holds a preference indistinguishable from one
 * picked in the menu, which later availability changes may not touch.
 *
 * Nothing is adopted while detection is still in flight, because the whole installed catalog is
 * `useAvailableAgents`' loading answer and adopting from it could store an agent no session could
 * be opened on. An installation that reaches none keeps `null` and adopts the first one that
 * arrives, which is the ordinary first-run sequence: agent processes are still completing their
 * handshake while the shell paints.
 */
export function useDefaultAgentAdoption(): void {
  const agentCli = useSettingsStore((state) => state.settings.agentCli);
  const updateSettings = useSettingsStore((state) => state.updateSettings);
  const { data: statuses } = useAgentRuntimeStatus();
  const availableAgents = useAvailableAgents();
  // Catalog order is the backend's stable identifier order, so which agent a fresh installation
  // lands on does not depend on how quickly each one answered.
  const adoptable = statuses === undefined ? undefined : availableAgents[0];

  useEffect(() => {
    if (agentCli !== null || adoptable === undefined) return;
    updateSettings({ agentCli: adoptable.agentRef });
  }, [agentCli, adoptable, updateSettings]);
}
