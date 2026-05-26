import { Avatar, AvatarFallback, AvatarImage, HoverCard, HoverCardContent, HoverCardTrigger } from "@ora/ui";
import { CalendarDays, Link } from "lucide-react";
import { Section, Row } from "./shared";

export default function HoverCardPage() {
  return (
    <>
      <Section title="Hover Card">
        <Row label="profile">
          <HoverCard>
            <HoverCardTrigger asChild>
              <a
                href="#"
                className="text-[13px] font-medium text-primary underline-offset-4 hover:underline"
                onClick={(e) => e.preventDefault()}
              >
                @nextjs
              </a>
            </HoverCardTrigger>
            <HoverCardContent>
              <div className="flex gap-3">
                <Avatar>
                  <AvatarImage src="https://github.com/vercel.png" />
                  <AvatarFallback>VC</AvatarFallback>
                </Avatar>
                <div className="flex flex-col gap-1">
                  <p className="text-sm font-semibold text-fg">@nextjs</p>
                  <p className="text-xs text-fg-secondary">
                    The React Framework — created and maintained by @vercel.
                  </p>
                  <div className="flex items-center gap-1 mt-1">
                    <CalendarDays className="h-3 w-3 text-fg-secondary" />
                    <span className="text-xs text-fg-secondary">Joined December 2021</span>
                  </div>
                </div>
              </div>
            </HoverCardContent>
          </HoverCard>
        </Row>

        <Row label="link card">
          <HoverCard>
            <HoverCardTrigger asChild>
              <a
                href="#"
                className="inline-flex items-center gap-1.5 text-[13px] text-fg-secondary hover:text-fg transition-colors"
                onClick={(e) => e.preventDefault()}
              >
                <Link className="h-3.5 w-3.5" />
                docs.example.com
              </a>
            </HoverCardTrigger>
            <HoverCardContent side="top">
              <div className="flex flex-col gap-2">
                <p className="text-xs font-semibold text-fg">Documentation</p>
                <p className="text-xs text-fg-secondary leading-relaxed">
                  Full API reference and guides for getting started with the
                  platform.
                </p>
                <span className="text-xs text-primary">docs.example.com →</span>
              </div>
            </HoverCardContent>
          </HoverCard>
        </Row>

        <Row label="side=right">
          <HoverCard>
            <HoverCardTrigger asChild>
              <span className="text-[13px] text-fg-secondary underline underline-offset-4 decoration-dashed cursor-default">
                hover for details
              </span>
            </HoverCardTrigger>
            <HoverCardContent side="right" className="w-48">
              <p className="text-xs text-fg-secondary">
                This card opens to the right side of the trigger element.
              </p>
            </HoverCardContent>
          </HoverCard>
        </Row>
      </Section>
    </>
  );
}
