import { useState } from "react";
import {
  Pagination,
  PaginationContent,
  PaginationEllipsis,
  PaginationItem,
  PaginationLink,
  PaginationNext,
  PaginationPrevious,
} from "@ora/ui";
import { Section, Row } from "./shared";

function ControlledPagination() {
  const [page, setPage] = useState(3);
  const total = 10;

  const go = (p: number) => (e: React.MouseEvent) => {
    e.preventDefault();
    if (p >= 1 && p <= total) setPage(p);
  };

  return (
    <Pagination>
      <PaginationContent>
        <PaginationItem>
          <PaginationPrevious href="#" onClick={go(page - 1)} />
        </PaginationItem>
        <PaginationItem>
          <PaginationLink href="#" isActive={page === 1} onClick={go(1)}>1</PaginationLink>
        </PaginationItem>
        {page > 3 && (
          <PaginationItem>
            <PaginationEllipsis />
          </PaginationItem>
        )}
        {page > 2 && (
          <PaginationItem>
            <PaginationLink href="#" onClick={go(page - 1)}>{page - 1}</PaginationLink>
          </PaginationItem>
        )}
        {page !== 1 && page !== total && (
          <PaginationItem>
            <PaginationLink href="#" isActive onClick={go(page)}>{page}</PaginationLink>
          </PaginationItem>
        )}
        {page < total - 1 && (
          <PaginationItem>
            <PaginationLink href="#" onClick={go(page + 1)}>{page + 1}</PaginationLink>
          </PaginationItem>
        )}
        {page < total - 2 && (
          <PaginationItem>
            <PaginationEllipsis />
          </PaginationItem>
        )}
        <PaginationItem>
          <PaginationLink href="#" isActive={page === total} onClick={go(total)}>{total}</PaginationLink>
        </PaginationItem>
        <PaginationItem>
          <PaginationNext href="#" onClick={go(page + 1)} />
        </PaginationItem>
      </PaginationContent>
    </Pagination>
  );
}

export default function PaginationPage() {
  return (
    <>
      <Section title="Pagination">
        <Row label="basic">
          <Pagination>
            <PaginationContent>
              <PaginationItem>
                <PaginationPrevious href="#" onClick={(e) => e.preventDefault()} />
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" onClick={(e) => e.preventDefault()}>1</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" isActive onClick={(e) => e.preventDefault()}>2</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" onClick={(e) => e.preventDefault()}>3</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationNext href="#" onClick={(e) => e.preventDefault()} />
              </PaginationItem>
            </PaginationContent>
          </Pagination>
        </Row>

        <Row label="with ellipsis">
          <Pagination>
            <PaginationContent>
              <PaginationItem>
                <PaginationPrevious href="#" onClick={(e) => e.preventDefault()} />
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" onClick={(e) => e.preventDefault()}>1</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationEllipsis />
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" onClick={(e) => e.preventDefault()}>4</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" isActive onClick={(e) => e.preventDefault()}>5</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" onClick={(e) => e.preventDefault()}>6</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationEllipsis />
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#" onClick={(e) => e.preventDefault()}>10</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationNext href="#" onClick={(e) => e.preventDefault()} />
              </PaginationItem>
            </PaginationContent>
          </Pagination>
        </Row>

        <Row label="controlled">
          <ControlledPagination />
        </Row>
      </Section>
    </>
  );
}
