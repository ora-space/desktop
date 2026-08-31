import { useTranslation } from "react-i18next";

/**
 * The landing copy shown above the composer while a session has no messages.
 * It is deliberately separate from the composer so ChatView can keep the
 * composer mounted across the empty/thread switch and animate it between them.
 */
export function LandingHeading() {
  const { t } = useTranslation();
  return (
    <div className="mb-7">
      <h1 className="text-2xl font-medium tracking-[-0.035em] text-foreground sm:text-[28px]">
        {t("chat.heading")}
      </h1>
      <p className="mt-2 text-sm text-muted-foreground">
        {t("chat.subheading")}
      </p>
    </div>
  );
}
