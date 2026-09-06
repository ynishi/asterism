/**
 * @vitest-environment happy-dom
 *
 * TabStrip rendering tests — specifically, the attributes an
 * extraction like this one has twice now dropped silently rather than
 * broken loudly: `ui-check` only flags a CSS selector that becomes
 * entirely unused, and a heading style, a divider colour, and an ARIA
 * role or a `type` attribute none of them fits that shape. This file
 * pins the markup contract directly, at the one place both callers
 * agree on.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import TabStrip from "./TabStrip.svelte";

afterEach(() => {
  cleanup();
});

function threeTabs(onSelect: (key: string) => void = () => {}) {
  return [
    { key: "a", label: "on the line", onSelect: () => onSelect("a") },
    { key: "b", label: "work", onSelect: () => onSelect("b") },
    { key: "c", label: "history", onSelect: () => onSelect("c") },
  ];
}

describe("TabStrip", () => {
  it("renders one tab per entry, under a tablist labelled by the caller", () => {
    render(TabStrip, { tabs: threeTabs(), active: "a", ariaLabel: "What to read" });
    const list = screen.getByRole("tablist", { name: "What to read" });
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(3);
    expect(list.contains(tabs[0])).toBe(true);
    expect(tabs.map((t) => t.textContent?.trim())).toEqual([
      "on the line",
      "work",
      "history",
    ]);
  });

  it("marks only the active key selected", () => {
    render(TabStrip, { tabs: threeTabs(), active: "b", ariaLabel: "What to read" });
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.getAttribute("aria-selected"))).toEqual([
      "false",
      "true",
      "false",
    ]);
  });

  it("gives every button an explicit type, so a caller inside a form is never at risk of an implicit submit", () => {
    render(TabStrip, { tabs: threeTabs(), active: "a", ariaLabel: "What to read" });
    for (const tab of screen.getAllByRole("tab")) {
      expect(tab.getAttribute("type")).toBe("button");
    }
  });

  it("calls the clicked tab's own onSelect, not the others'", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(TabStrip, { tabs: threeTabs(onSelect), active: "a", ariaLabel: "What to read" });
    await user.click(screen.getByRole("tab", { name: "history" }));
    expect(onSelect).toHaveBeenCalledExactlyOnceWith("c");
  });
});
