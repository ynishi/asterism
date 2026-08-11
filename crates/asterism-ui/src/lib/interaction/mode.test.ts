// Unit tests for the interaction mode stack (W1 slice).
import { beforeEach, describe, expect, it } from "vitest";

import { interaction } from "./mode.svelte";

function drain() {
  while (interaction.pop() !== null) {
    // pop until empty
  }
}

describe("interaction mode stack", () => {
  beforeEach(drain);

  it("starts empty: no top, no overlay", () => {
    expect(interaction.top).toBeNull();
    expect(interaction.overlayActive).toBe(false);
  });

  it("push sets top and overlayActive", () => {
    interaction.push("marquee");
    expect(interaction.top).toBe("marquee");
    expect(interaction.overlayActive).toBe(true);
    expect(interaction.has("marquee")).toBe(true);
  });

  it("push is idempotent per mode and re-raises to top", () => {
    interaction.push("marquee");
    interaction.push("cardMenu");
    interaction.push("marquee");
    expect(interaction.top).toBe("marquee");
    // No duplicate: removing once clears it entirely.
    interaction.remove("marquee");
    expect(interaction.has("marquee")).toBe(false);
    expect(interaction.top).toBe("cardMenu");
  });

  it("remove of a non-registered mode is a no-op", () => {
    interaction.push("cardMenu");
    interaction.remove("marquee");
    expect(interaction.top).toBe("cardMenu");
  });

  it("pop returns and removes the top mode, LIFO", () => {
    interaction.push("detail");
    interaction.push("cardMenu");
    expect(interaction.pop()).toBe("cardMenu");
    expect(interaction.pop()).toBe("detail");
    expect(interaction.pop()).toBeNull();
    expect(interaction.overlayActive).toBe(false);
  });
});
