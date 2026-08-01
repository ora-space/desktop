import {
  LocalTransportError,
  RemoteContractError,
  UnknownRemoteError,
} from "@ora/contracts";
import type { TFunction } from "i18next";
import type { TranslationKey } from "./i18n-instance";

/** Localizes every remote or local transport failure without displaying technical Error.message. */
export function localizeContractError(error: unknown, t: TFunction): string {
  if (error instanceof RemoteContractError) {
    return t(`errors.${error.code}` as TranslationKey, {
      ...error.payload.params,
      requestId: error.requestId,
    });
  }
  if (error instanceof UnknownRemoteError) {
    return t("errors.unknown", { requestId: error.requestId });
  }
  if (error instanceof LocalTransportError) {
    return t(`errors.transport.${error.kind}` as TranslationKey);
  }
  return t("errors.transport.malformed_response");
}
