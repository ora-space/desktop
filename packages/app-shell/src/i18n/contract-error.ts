import {
  LocalTransportError,
  RemoteContractError,
  UnknownRemoteError,
} from "@ora/contracts";
import type { TFunction } from "i18next";
import {
  activeLocale,
  translationResources,
  type Locale,
  type TranslationKey,
} from "./i18n-instance";

function formatInterpolationParams(
  params: Record<string, unknown>,
  lng: Locale,
): Record<string, unknown> {
  const separator = lng === "zh-CN" ? "、" : ", ";
  return Object.fromEntries(
    Object.entries(params).map(([key, value]) => [
      key,
      Array.isArray(value) && value.every((item) => typeof item === "string")
        ? value.join(separator)
        : value,
    ]),
  );
}

/** Localizes every remote or local transport failure without displaying technical Error.message. */
export function localizeContractError(error: unknown, t: TFunction): string {
  const lng = activeLocale();
  if (error instanceof RemoteContractError) {
    const key = `errors.${error.code}`;
    if (key in translationResources[lng]) {
      return t(key as TranslationKey, {
        ...formatInterpolationParams(
          error.payload.params as Record<string, unknown>,
          lng,
        ),
        requestId: error.requestId,
        lng,
      });
    }
    return t("errors.unknown", { requestId: error.requestId, lng });
  }
  if (error instanceof UnknownRemoteError) {
    return t("errors.unknown", { requestId: error.requestId, lng });
  }
  if (error instanceof LocalTransportError) {
    const key = `errors.transport.${error.kind}`;
    if (key in translationResources[lng]) {
      return t(key as TranslationKey, { lng });
    }
    return t("errors.transport.malformed_response", { lng });
  }
  return t("errors.transport.malformed_response", { lng });
}

/**
 * Describes a failed chat request in the active locale for the conversation transcript.
 *
 * Contract and transport failures carry a code the user can act on, so they are translated
 * instead of showing "Remote Ora request failed with <code> (<request id>)". Any other error
 * (an agent's own failure text, for instance) already reads as a sentence and is kept as is.
 */
export function describeChatError(error: unknown, t: TFunction): string {
  if (
    error instanceof RemoteContractError ||
    error instanceof UnknownRemoteError ||
    error instanceof LocalTransportError
  ) {
    return localizeContractError(error, t);
  }
  return error instanceof Error ? error.message : String(error);
}

/** Identifies failures whose request ID is useful only together with backend diagnostics. */
export function hasDiagnosticRequestId(error: unknown): boolean {
  return (
    (error instanceof RemoteContractError && error.code === "internal_error") ||
    error instanceof UnknownRemoteError
  );
}
