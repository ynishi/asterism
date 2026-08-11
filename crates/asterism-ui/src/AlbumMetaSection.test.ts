/**
 * @vitest-environment happy-dom
 *
 * AlbumMetaSection wiring tests — the layer between a person's finger
 * and the declare verb.
 *
 * `lib/album-meta.test.ts` next door pins how a bag is read. It cannot
 * say whether the panel sends what the person typed: a form whose
 * remove button passed the wrong key, or whose retraction sent `""`
 * instead of nothing, would pass every one of those tests and still
 * delete the wrong statement — or refuse to delete anything, since the
 * server rejects a blank value outright.
 *
 * Scope is the wiring. Layout and copy are not asserted; the queries go
 * through roles and labels, so this tests what the section *offers*.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import type { AssetDto } from "./bindings";
import { api } from "./lib/api";
import type { AlbumMetaStatement } from "./lib/album-meta";
import AlbumMetaSection from "./AlbumMetaSection.svelte";

vi.mock("./lib/api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

function statement(key: string, value: string): AlbumMetaStatement {
  return {
    key,
    value,
    source: "manual",
    operator: null,
    declaredAtMs: 1_785_000_000_000,
  };
}

function asset(id: string): AssetDto {
  // Only the id is read by the callback under test; the rest of the DTO
  // is the panel owner's business.
  return { id } as AssetDto;
}

beforeEach(() => {
  cleanup();
  apiMock.mockReset();
  apiMock.mockResolvedValue(asset("a1"));
});

// The suite carries no jest-dom matchers, so state is read off the
// elements themselves rather than through `toBeDisabled` / `toHaveValue`.
function submitButton(): HTMLButtonElement {
  return screen.getByRole("button", { name: "State" }) as HTMLButtonElement;
}

function mount(statements: AlbumMetaStatement[] = []) {
  const onChanged = vi.fn();
  render(AlbumMetaSection, {
    props: { assetId: "a1", statements, onChanged },
  });
  return { onChanged };
}

describe("AlbumMetaSection", () => {
  it("states what the person typed under the name they gave", async () => {
    const user = userEvent.setup();
    const { onChanged } = mount();

    await user.type(screen.getByLabelText("Statement name"), "workflow-id");
    await user.type(screen.getByLabelText("Statement value"), "wf-1");
    await user.click(screen.getByRole("button", { name: "State" }));

    expect(apiMock).toHaveBeenCalledWith("asset_declare_meta", {
      command: {
        asset_id: "a1",
        key: "workflow-id",
        value: "wf-1",
        operator_ai: null,
      },
    });
    expect(onChanged).toHaveBeenCalledWith(asset("a1"));
  });

  it("retracts by sending no value, not an empty one", async () => {
    const user = userEvent.setup();
    mount([statement("plate", "offwhite"), statement("workflow-id", "wf-1")]);

    // The *second* row on purpose. Pressing the first would agree with
    // a button wired to `statements[0]`, so the axis under test has to
    // disagree with that default before the assertion means anything.
    await user.click(
      screen.getByRole("button", { name: "Take back workflow-id" }),
    );

    // `""` is refused by the server on purpose — a caller that sends it
    // has almost always failed to build the value it meant. So the
    // retraction has to be an absent value, and it has to name the row
    // whose button was pressed.
    expect(apiMock).toHaveBeenCalledWith("asset_declare_meta", {
      command: {
        asset_id: "a1",
        key: "workflow-id",
        value: null,
        operator_ai: null,
      },
    });
  });

  it("will not send a name the server would refuse", async () => {
    const user = userEvent.setup();
    mount();

    await user.type(screen.getByLabelText("Statement name"), "Workflow");
    await user.type(screen.getByLabelText("Statement value"), "wf-1");

    expect(submitButton().disabled).toBe(true);
    expect(screen.getByText(/lowercase/)).toBeTruthy();
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("will not send a half-filled row", async () => {
    const user = userEvent.setup();
    mount();

    expect(submitButton().disabled).toBe(true);

    await user.type(screen.getByLabelText("Statement name"), "plate");
    expect(submitButton().disabled).toBe(true);

    await user.type(screen.getByLabelText("Statement value"), "offwhite");
    expect(submitButton().disabled).toBe(false);
  });

  it("keeps the draft when the write is refused", async () => {
    const user = userEvent.setup();
    apiMock.mockRejectedValue("Validation: nope");
    mount();

    await user.type(screen.getByLabelText("Statement name"), "plate");
    await user.type(screen.getByLabelText("Statement value"), "offwhite");
    await user.click(screen.getByRole("button", { name: "State" }));

    // Clearing the inputs on a refusal would make the person retype
    // something the server has not accepted yet.
    const name = screen.getByLabelText("Statement name") as HTMLInputElement;
    expect(name.value).toBe("plate");
    expect(screen.getByText(/nope/)).toBeTruthy();
  });

  it("shows the statements it was handed, and says so when there are none", () => {
    mount([statement("workflow-id", "wf-1")]);
    expect(screen.getByText("workflow-id")).toBeTruthy();
    expect(screen.getByText("wf-1")).toBeTruthy();

    cleanup();
    mount();
    expect(screen.getByText("nothing stated yet")).toBeTruthy();
  });
});
