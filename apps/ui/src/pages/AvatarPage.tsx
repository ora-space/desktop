import { Avatar, AvatarFallback, AvatarImage } from "@ora/ui";
import { User } from "lucide-react";
import { Section, Row } from "./shared";

export default function AvatarPage() {
  return (
    <Section title="Avatar">
      <Row label="default">
        <Avatar>
          <AvatarImage src="https://github.com/shadcn.png" alt="@shadcn" />
          <AvatarFallback>CN</AvatarFallback>
        </Avatar>
        <Avatar>
          <AvatarFallback>
            <User className="h-5 w-5" />
          </AvatarFallback>
        </Avatar>
      </Row>
    </Section>
  );
}
