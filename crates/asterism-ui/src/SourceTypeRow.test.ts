/**
 * @vitest-environment happy-dom
 *
 * SourceTypeRow wiring tests — the layer between a person's finger and
 * the source-type verb (#108).
 *
 * The backend refuses unknown terms and reads an absent term as the
 * retraction; what these pin is that the row cannot send anything the
 * backend would refuse (the select is closed over the five terms) and
 * that Retract sends nothing rather than `""`. Layout and copy are not
 * asserted beyond the three states' distinguishing text; queries go
 * through roles and labels.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import type { AssetDto, AssetSourceTypeDto } from "./bindings";
import { api } from "./lib/api";
import SourceTypeRow from "./SourceTypeRow.svelte";

vi.mock("./lib/api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

function reading(
  overrides: Partial<AssetSourceTypeDto> = {},
): AssetSourceTypeDto {
  return {
    asset_id: "a1",
    evidence: null,
    evidence_pending: false,
    asserted: null,
    ...overrides,
  };
}

function asset(id: string): AssetDto {
  // Only the id is read by the callback under test; the rest of the
  // DTO is the pane's business.
  return { id } as AssetDto;
}

beforeEach(() => {
  cleanup();
  apiMock.mockReset();
});

describe("SourceTypeRow", () => {
  it("shows an assertion with who and when, and Retract sends nothing", async () => {
    apiMock.mockResolvedValue(
      reading({
        asserted: {
          source_type: "trainedAlgorithmicMedia",
          operator: "claude",
          declared_at_ms: 1_785_000_000_000,
        },
      }),
    );
    render(SourceTypeRow, { assetId: "a1", onChanged: vi.fn() });

    expect(
      await screen.findByText("trainedAlgorithmicMedia"),
    ).toBeTruthy();
    expect(screen.getByText(/via claude/)).toBeTruthy();

    apiMock.mockReset();
    apiMock.mockResolvedValueOnce(asset("a1"));
    apiMock.mockResolvedValueOnce(reading());
    await userEvent.click(screen.getByRole("button", { name: "Retract" }));

    const [, args] = apiMock.mock.calls[0];
    expect(apiMock.mock.calls[0][0]).toBe("asset_declare_source_type");
    // Absent, not "": the server reads a missing term as the
    // retraction and refuses a blank outright.
    expect(args).toEqual({
      command: { asset_id: "a1", source_type: null, operator_ai: null },
    });
  });

  it("labels container evidence as the container's, behind Override…", async () => {
    apiMock.mockResolvedValue(reading({ evidence: "digitalCapture" }));
    render(SourceTypeRow, { assetId: "a1", onChanged: vi.fn() });

    expect(await screen.findByText("digitalCapture")).toBeTruthy();
    expect(screen.getByText("from the container")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Override…" }),
    ).toBeTruthy();
    // Read-only until overridden: no Retract on the container's own word.
    expect(screen.queryByRole("button", { name: "Retract" })).toBeNull();
  });

  it("offers exactly the five IPTC terms and sends the chosen one", async () => {
    const onChanged = vi.fn();
    apiMock.mockResolvedValue(reading());
    render(SourceTypeRow, { assetId: "a1", onChanged });

    expect(
      await screen.findByText("container declares nothing"),
    ).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Assert…" }));

    const select = screen.getByRole("combobox", {
      name: "Digital source type",
    });
    const options = Array.from(select.querySelectorAll("option")).map(
      (option) => option.value,
    );
    expect(options).toEqual([
      "digitalCapture",
      "humanEdits",
      "trainedAlgorithmicMedia",
      "compositeWithTrainedAlgorithmicMedia",
      "algorithmicMedia",
    ]);

    apiMock.mockReset();
    apiMock.mockResolvedValueOnce(asset("a1"));
    apiMock.mockResolvedValueOnce(
      reading({
        asserted: {
          source_type: "humanEdits",
          operator: null,
          declared_at_ms: 1_785_000_000_000,
        },
      }),
    );
    await fireEvent.change(select, { target: { value: "humanEdits" } });
    await userEvent.click(screen.getByRole("button", { name: "Assert" }));

    expect(apiMock.mock.calls[0]).toEqual([
      "asset_declare_source_type",
      {
        command: {
          asset_id: "a1",
          source_type: "humanEdits",
          operator_ai: null,
        },
      },
    ]);
    // The verb answers with the whole row, handed up so the pane's
    // cached copy of `extra` does not go stale.
    await waitFor(() => expect(onChanged).toHaveBeenCalledWith(asset("a1")));
    // And the row re-reads its own state rather than inferring it.
    expect(await screen.findByText("humanEdits")).toBeTruthy();
  });

  it("keeps 'not yet read' apart from 'declares nothing'", async () => {
    apiMock.mockResolvedValue(reading({ evidence_pending: true }));
    render(SourceTypeRow, { assetId: "a1", onChanged: vi.fn() });

    expect(
      await screen.findByText("container not yet read"),
    ).toBeTruthy();
    expect(screen.queryByText("container declares nothing")).toBeNull();
  });
});
