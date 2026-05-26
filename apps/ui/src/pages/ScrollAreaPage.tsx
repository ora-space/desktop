import { ScrollArea, ScrollBar } from "@ora/ui";
import { Section, Row } from "./shared";

const TAGS = Array.from({ length: 50 }, (_, i) => `Item ${i + 1}`);

const ARTWORKS = [
  { title: "Orca", artist: "Orcinus orca" },
  { title: "Moonlight", artist: "Jean-Baptiste Mondino" },
  { title: "Starry Night", artist: "Vincent van Gogh" },
  { title: "The Persistence of Memory", artist: "Salvador Dalí" },
  { title: "Girl with a Pearl Earring", artist: "Johannes Vermeer" },
  { title: "The Scream", artist: "Edvard Munch" },
  { title: "Water Lilies", artist: "Claude Monet" },
  { title: "The Birth of Venus", artist: "Sandro Botticelli" },
  { title: "Las Meninas", artist: "Diego Velázquez" },
  { title: "The Kiss", artist: "Gustav Klimt" },
  { title: "A Sunday on La Grande Jatte", artist: "Georges Seurat" },
  { title: "Guernica", artist: "Pablo Picasso" },
];

export default function ScrollAreaPage() {
  return (
    <>
      <Section title="Scroll Area">
        <Row label="vertical">
          <ScrollArea className="h-48 w-48 rounded-md border border-border">
            <div className="p-3">
              <p className="text-xs font-semibold text-fg mb-2">Tags</p>
              {TAGS.map((tag) => (
                <div
                  key={tag}
                  className="py-1 text-[13px] text-fg-secondary border-b border-border-subtle last:border-0"
                >
                  {tag}
                </div>
              ))}
            </div>
          </ScrollArea>
        </Row>

        <Row label="horizontal">
          <ScrollArea className="w-72 rounded-md border border-border">
            <div className="flex gap-3 p-3">
              {ARTWORKS.map((a) => (
                <div
                  key={a.title}
                  className="flex shrink-0 flex-col gap-1 w-28"
                >
                  <div className="h-20 rounded-sm bg-bg-subtle border border-border flex items-center justify-center text-xs text-fg-secondary text-center px-2">
                    {a.title}
                  </div>
                  <p className="text-xs text-fg truncate">{a.title}</p>
                  <p className="text-xs text-fg-secondary truncate">{a.artist}</p>
                </div>
              ))}
            </div>
            <ScrollBar orientation="horizontal" />
          </ScrollArea>
        </Row>

        <Row label="both axes">
          <ScrollArea className="h-40 w-64 rounded-md border border-border">
            <div className="p-3 w-96">
              <p className="text-xs font-semibold text-fg mb-2">Wide content</p>
              {Array.from({ length: 20 }, (_, i) => (
                <div key={i} className="py-0.5 text-[13px] text-fg-secondary whitespace-nowrap">
                  {`Row ${i + 1}: This line intentionally extends beyond the container width`}
                </div>
              ))}
            </div>
            <ScrollBar orientation="horizontal" />
          </ScrollArea>
        </Row>
      </Section>
    </>
  );
}
