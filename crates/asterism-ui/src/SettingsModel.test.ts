/**
 * @vitest-environment happy-dom
 *
 * SettingsModel — the model section of the settings stack (#130).
 *
 * Three things here can be wrong in a way no backend test would see.
 * The panel can report a state the backend did not answer with: a head
 * that is not bound, a restart asked for when nothing moved. It can
 * reword a job's verdict, which is the one string whose value is being
 * the backend's own — "promoted" against "not promoted" is the whole
 * outcome of a training run, and a refused pull says why in a sentence
 * nothing else knows. And it can send a paste-box's contents that were
 * never an object.
 *
 * So the reads are mocked at `api`, the job's answer is delivered
 * through a mocked `job:progress:{task_id}` listener, and each test
 * asserts what reached the screen rather than what was rendered around
 * it.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import type { HeadStatusDto, VisualModelStatusDto } from "./bindings";
import { api } from "./lib/api";
import SettingsModel from "./SettingsModel.svelte";

vi.mock("./lib/api", () => ({ api: vi.fn() }));

type ProgressHandler = (event: {
  payload: { current?: number; total?: number | null; message?: string };
}) => void;

const handlers = new Map<string, ProgressHandler>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: ProgressHandler) => {
    handlers.set(name, handler);
    return () => handlers.delete(name);
  }),
}));

const apiMock = vi.mocked(api);

const encoder: VisualModelStatusDto = {
  model_id: "siglip2-base-patch16-256",
  dim: 768,
  preprocess_ver: 1,
};

function status(over: Partial<HeadStatusDto> = {}): HeadStatusDto {
  return {
    promoted: null,
    bound: null,
    restart_required: false,
    run: null,
    readiness: {
      rulings: 0,
      tags_with_rulings: 0,
      tags_ready: 0,
      min_rulings_per_class: 4,
    },
    ...over,
  };
}

/** Answers the two reads; `head` may change between reloads. */
function wireReads(
  model: VisualModelStatusDto,
  heads: HeadStatusDto[],
): void {
  let call = 0;
  apiMock.mockImplementation(async (cmd: string) => {
    if (cmd === "visual_model_status") return model as never;
    if (cmd === "head_status") {
      const next = heads[Math.min(call, heads.length - 1)];
      call += 1;
      return next as never;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
}

beforeEach(() => {
  cleanup();
  handlers.clear();
  apiMock.mockReset();
});

describe("SettingsModel", () => {
  it("shows the bound encoder, the head that scores, and the ruling floor", async () => {
    wireReads(encoder, [
      status({
        promoted: "head-v2-1a2b3c4d",
        bound: "head-v2-1a2b3c4d",
        run: {
          model_id: "siglip2-base-patch16-256",
          dim: 768,
          preprocess_ver: 1,
          trained_tags: 3,
          rulings_used: 42,
          held_out: 10,
          candidate_correct: 8,
          baseline_correct: 6,
          trained_at_ms: 1_700_000_000_000,
        },
        readiness: {
          rulings: 42,
          tags_with_rulings: 7,
          tags_ready: 3,
          min_rulings_per_class: 4,
        },
      }),
    ]);
    render(SettingsModel);

    expect(await screen.findByText("head-v2-1a2b3c4d")).toBeTruthy();
    expect(screen.getByText("siglip2-base-patch16-256")).toBeTruthy();
    expect(
      screen.getByText(/768 dimensions · preprocess rev 1/),
    ).toBeTruthy();
    // The eval that promoted it, both sides of the comparison.
    expect(
      screen.getByText(/held-out 10 — this head 8\s+vs zero-shot 6/),
    ).toBeTruthy();
    expect(
      screen.getByText(
        /42 ruling\(s\) across 7 tag\(s\); 3 clear the\s+training floor of 4 per class/,
      ),
    ).toBeTruthy();
    // Nothing moved, so nothing asks for a relaunch.
    expect(screen.queryByText("restart")).toBeNull();
  });

  it("says so when no model is bound, and offers neither verb", async () => {
    wireReads({ model_id: null, dim: null, preprocess_ver: null }, [status()]);
    render(SettingsModel);

    expect(await screen.findByText(/No model bound/)).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Train now" }).hasAttribute("disabled"),
    ).toBe(true);
    expect(
      screen.getByRole("button", { name: "Install" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("surfaces the training verdict verbatim and re-reads what scores", async () => {
    const promoted = status({
      promoted: "head-v3-deadbeef",
      bound: null,
      restart_required: true,
    });
    let call = 0;
    apiMock.mockImplementation(async (cmd: string) => {
      if (cmd === "visual_model_status") return encoder as never;
      if (cmd === "head_status") {
        call += 1;
        return (call === 1 ? status() : promoted) as never;
      }
      if (cmd === "train_tag_head") return "task-7" as never;
      throw new Error(`unexpected command ${cmd}`);
    });
    render(SettingsModel);

    await screen.findByText("zero-shot");
    await userEvent.click(screen.getByRole("button", { name: "Train now" }));
    await waitFor(() =>
      expect(handlers.has("job:progress:task-7")).toBe(true),
    );

    const verdict =
      "head_train: head-v3-deadbeef: 3 tag(s) trained on 42 ruling(s); " +
      "held-out 10 — candidate 8 vs zero-shot 6; promoted — the pointer " +
      "is set; restart applies it";
    handlers.get("job:progress:task-7")!({
      payload: { current: 1, total: 1, message: verdict },
    });

    // The handler's sentence, not a "done" of the panel's own.
    expect(await screen.findByText(verdict)).toBeTruthy();
    // The pointer moved, and the badge follows the re-read rather than
    // the click.
    expect(await screen.findByText("restart")).toBeTruthy();
  });

  it("surfaces a refused pull as the job worded it", async () => {
    wireReads(encoder, [status()]);
    apiMock.mockImplementation(async (cmd: string) => {
      if (cmd === "visual_model_status") return encoder as never;
      if (cmd === "head_status") return status() as never;
      if (cmd === "pull_tag_head") return "task-9" as never;
      throw new Error(`unexpected command ${cmd}`);
    });
    render(SettingsModel);
    await screen.findByText("zero-shot");

    const box = screen.getByPlaceholderText("the head artifact, as fetched");
    await userEvent.type(box, '{{"schema":"asterism-tag-head-v1"}');
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(handlers.has("job:progress:task-9")).toBe(true),
    );

    const refusal =
      "head_pull failed: head head-v4-c0ffee was trained under other/512d/p1, " +
      "the bound encoder is siglip2-base-patch16-256/768d/p1 — a head scores " +
      "only against the vectors it learned from";
    handlers.get("job:progress:task-9")!({
      payload: { current: 1, total: 1, message: refusal },
    });

    expect(await screen.findByText(refusal)).toBeTruthy();
  });

  it("unlocks on the per-kind tick when the task's own event never lands", async () => {
    apiMock.mockImplementation(async (cmd: string) => {
      if (cmd === "visual_model_status") return encoder as never;
      if (cmd === "head_status") return status() as never;
      if (cmd === "train_tag_head") return "task-11" as never;
      throw new Error(`unexpected command ${cmd}`);
    });
    render(SettingsModel);
    await screen.findByText("zero-shot");

    const train = screen.getByRole("button", { name: "Train now" });
    await userEvent.click(train);
    await waitFor(() => expect(handlers.has("jobs:tick")).toBe(true));
    expect(train.hasAttribute("disabled")).toBe(true);

    // Delivery of `job:progress:{task_id}` is best-effort; the section
    // must not stay busy for the rest of the session when one is lost.
    handlers.get("jobs:tick")!({ payload: { kind: "head_train" } as never });
    await waitFor(() => expect(train.hasAttribute("disabled")).toBe(false));
  });

  it("refuses a paste that is not an object before enqueuing anything", async () => {
    wireReads(encoder, [status()]);
    render(SettingsModel);
    await screen.findByText("zero-shot");

    const box = screen.getByPlaceholderText("the head artifact, as fetched");
    await userEvent.type(box, "not json at all");
    await userEvent.click(screen.getByRole("button", { name: "Install" }));

    expect(await screen.findByText(/That is not JSON/)).toBeTruthy();
    expect(
      apiMock.mock.calls.some(([cmd]) => cmd === "pull_tag_head"),
    ).toBe(false);
  });
});
