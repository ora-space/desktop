import { render } from "@testing-library/react";
import { Dialog, DialogContent, DialogTitle } from "@ora/ui";
import { beforeEach, describe, expect, it } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { PlatformProvider } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { SurfaceHost } from "./surface-host";

/** Two independently controlled dialogs standing in for nested overlays. */
function Dialogs({ outer, inner }: { outer: boolean; inner: boolean }) {
  return (
    <>
      <Dialog open={outer}>
        <DialogContent>
          <DialogTitle>Outer</DialogTitle>
        </DialogContent>
      </Dialog>
      <Dialog open={inner}>
        <DialogContent>
          <DialogTitle>Inner</DialogTitle>
        </DialogContent>
      </Dialog>
    </>
  );
}

beforeEach(() => {
  useSurfaceStore.setState({
    embeddedSupported: true,
    records: {},
    failures: {},
    sidePanelInstance: 1,
  });
});

describe("surface occlusion", () => {
  it("re-shows the surface only once the last overlay has closed", () => {
    const host = createSurfaceTestPlatform({ embedded: true });
    const scene = (outer: boolean, inner: boolean) => (
      <AppI18nProvider>
        <PlatformProvider adapter={host.platform}>
          <SurfaceHost instance={1} />
          <Dialogs outer={outer} inner={inner} />
        </PlatformProvider>
      </AppI18nProvider>
    );
    const { rerender } = render(scene(false, false));
    expect(host.surfaces.setVisible.mock.calls).toEqual([[1, true]]);

    rerender(scene(true, false));
    rerender(scene(true, true));
    expect(host.surfaces.setVisible.mock.calls).toEqual([
      [1, true],
      [1, false],
    ]);

    rerender(scene(false, true));
    expect(host.surfaces.setVisible.mock.calls).toEqual([
      [1, true],
      [1, false],
    ]);

    rerender(scene(false, false));
    expect(host.surfaces.setVisible.mock.calls).toEqual([
      [1, true],
      [1, false],
      [1, true],
    ]);
  });
});
