import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger, Button } from "@ora/ui";
import { Info, Settings, Trash2 } from "lucide-react";
import { Section, Row } from "./shared";

export default function TooltipPage() {
  return (
    <TooltipProvider>
      <Section title="Tooltip">
        <Row label="default">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline" size="md">Hover me</Button>
            </TooltipTrigger>
            <TooltipContent>This is a tooltip</TooltipContent>
          </Tooltip>
        </Row>
        <Row label="sides">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline" size="sm">Top</Button>
            </TooltipTrigger>
            <TooltipContent side="top">Tooltip on top</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline" size="sm">Right</Button>
            </TooltipTrigger>
            <TooltipContent side="right">Tooltip on right</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline" size="sm">Bottom</Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">Tooltip on bottom</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline" size="sm">Left</Button>
            </TooltipTrigger>
            <TooltipContent side="left">Tooltip on left</TooltipContent>
          </Tooltip>
        </Row>
        <Row label="icon buttons">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon">
                <Info className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>More information</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon">
                <Settings className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Open settings</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon">
                <Trash2 className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Delete item</TooltipContent>
          </Tooltip>
        </Row>
        <Row label="delay">
          <Tooltip delayDuration={0}>
            <TooltipTrigger asChild>
              <Button variant="outline" size="sm">No delay</Button>
            </TooltipTrigger>
            <TooltipContent>Appears instantly</TooltipContent>
          </Tooltip>
          <Tooltip delayDuration={700}>
            <TooltipTrigger asChild>
              <Button variant="outline" size="sm">Long delay</Button>
            </TooltipTrigger>
            <TooltipContent>Appears after 700ms</TooltipContent>
          </Tooltip>
        </Row>
        <Row label="disabled trigger">
          <Tooltip>
            <TooltipTrigger asChild>
              <span tabIndex={0}>
                <Button variant="outline" size="md" disabled>
                  Disabled button
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>Explains why it's disabled</TooltipContent>
          </Tooltip>
        </Row>
      </Section>
    </TooltipProvider>
  );
}
