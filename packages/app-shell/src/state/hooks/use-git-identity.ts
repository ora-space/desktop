import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type { ContractsClient } from "@ora/contracts";
import type { CurrentUser } from "../../lib/types";
import { identityKeys } from "../data/identity";

/**
 * Resolves the sidebar user from the host's global Git identity.
 *
 * The name/email come from `git config --global`, read once per session and
 * cached indefinitely since the global config rarely changes mid-session. Until
 * the value arrives - or when the key is unset or the read fails - the name
 * falls back to a neutral placeholder and the email is left empty so the profile
 * can hide its second line. Pass `enabled: false` to skip the read entirely
 * (for example when an explicit user is injected in tests).
 */
export function useGitIdentityUser(
  client: ContractsClient,
  enabled: boolean,
): CurrentUser {
  const { t } = useTranslation();
  const { data } = useQuery({
    queryKey: identityKeys.gitIdentity,
    queryFn: () => client.gitIdentity.get({}),
    staleTime: Number.POSITIVE_INFINITY,
    enabled,
  });

  return {
    name: data?.name?.trim() || t("account.unknownIdentity"),
    email: data?.email?.trim() ?? "",
  };
}
