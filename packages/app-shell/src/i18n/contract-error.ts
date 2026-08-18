import {
  LocalTransportError,
  RemoteContractError,
  UnknownRemoteError,
} from "@ora/contracts";
import type { TFunction } from "i18next";
import {
  activeLocale,
  translationResources,
  type TranslationKey,
} from "./i18n-instance";

/** Localizes every remote or local transport failure without displaying technical Error.message. */
export function localizeContractError(error: unknown, t: TFunction): string {
  const lng = activeLocale();
  if (error instanceof RemoteContractError) {
    const key = `errors.${error.code}`;
    if (key in translationResources[lng]) {
      return t(key as TranslationKey, {
        ...error.payload.params,
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
