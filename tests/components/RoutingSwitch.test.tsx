import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Provider } from "@/types";
import { RoutingSwitch } from "@/components/providers/mode/RoutingSwitch";

const provider = (id: string): Provider => ({
  id,
  name: id,
  settingsConfig: {},
});

function renderSwitch(overrides: Record<string, unknown> = {}) {
  const onChange = vi.fn();
  render(
    <RoutingSwitch
      app="claude"
      active="direct"
      directProvider={provider("relay")}
      routeProvider={undefined}
      onChange={onChange}
      {...overrides}
    />,
  );
  return onChange;
}

const toggle = () =>
  screen.getByRole("switch", { name: "mode.routingSwitchAria" });

describe("RoutingSwitch", () => {
  it("is off while direct and turns routing on", () => {
    const onChange = renderSwitch();
    expect(toggle()).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle());
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("is on while routing and turns it off", () => {
    const onChange = renderSwitch({ active: "route" });
    expect(toggle()).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle());
    expect(onChange).toHaveBeenCalledWith(false);
  });

  it("shows the current route target when routing", () => {
    renderSwitch({
      active: "route",
      directProvider: provider("relay"),
      routeProvider: provider("backup"),
    });
    expect(screen.getByText("mode.names.route · backup")).toBeInTheDocument();
  });

  it("disables the switch with a reason when the direct provider cannot route", () => {
    renderSwitch({ disabledReason: "providerCard.reason.noRoute" });
    expect(toggle()).toBeDisabled();
  });

  it("stays disabled while a change is pending", () => {
    renderSwitch({ pending: true });
    expect(toggle()).toBeDisabled();
  });
});
