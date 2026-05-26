import { Button, Empty } from "@ora/ui";
import { Inbox, FileSearch, Users, FolderOpen } from "lucide-react";
import { Section, Row } from "./shared";

export default function EmptyPage() {
  return (
    <>
      <Section title="Empty">
        <Row label="default">
          <div className="w-72 rounded-md border border-border">
            <Empty
              icon={<Inbox className="h-5 w-5" />}
              title="No messages"
              description="You don't have any messages yet. Start a conversation to get going."
              action={<Button size="sm">New Message</Button>}
            />
          </div>
        </Row>

        <Row label="no action">
          <div className="w-72 rounded-md border border-border">
            <Empty
              icon={<FileSearch className="h-5 w-5" />}
              title="No results"
              description="Try adjusting your search or filters to find what you're looking for."
            />
          </div>
        </Row>

        <Row label="no icon">
          <div className="w-72 rounded-md border border-border">
            <Empty
              title="No teammates"
              description="Invite your team members to collaborate on this project."
              action={
                <Button variant="secondary" size="sm">
                  <Users className="h-3.5 w-3.5" />
                  Invite members
                </Button>
              }
            />
          </div>
        </Row>

        <Row label="minimal">
          <div className="w-72 rounded-md border border-border">
            <Empty icon={<FolderOpen className="h-5 w-5" />} title="Empty folder" />
          </div>
        </Row>
      </Section>
    </>
  );
}
