"use client"

import { useTheme } from "next-themes"
import { Toaster as Sonner, toast, type ToasterProps } from "sonner"
import { IconCircleCheck, IconInfoCircle, IconAlertTriangle, IconAlertOctagon, IconLoader2 } from "@tabler/icons-react"

const Toaster = ({ ...props }: ToasterProps) => {
  const { theme = "system" } = useTheme()

  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group"
      icons={{
        success: (
          <IconCircleCheck className="size-4" />
        ),
        info: (
          <IconInfoCircle className="size-4" />
        ),
        warning: (
          <IconAlertTriangle className="size-4" />
        ),
        error: (
          <IconAlertOctagon className="size-4" />
        ),
        loading: (
          <IconLoader2 className="size-4 animate-spin" />
        ),
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
        } as React.CSSProperties
      }
      toastOptions={{
        classNames: {
          // Lay the toast out as a row so the close button can sit inline at the end.
          toast: "cn-toast flex items-center gap-2",
          // Pull sonner's default corner-anchored close button back into the content
          // flow. It is the first DOM child, so `order-last` moves it visually after
          // the text; `ml-auto` then pins it to the right edge while the icon and text
          // stay left. Always visible, and restyled as a plain muted icon.
          closeButton:
            "!static !order-last !ml-auto !self-center flex !size-5 shrink-0 items-center justify-center !transform-none !border-none !bg-transparent !opacity-100 !text-muted-foreground hover:!text-foreground [&>svg]:!size-4",
        },
      }}
      {...props}
    />
  )
}

export { Toaster, toast }
