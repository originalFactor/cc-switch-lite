import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProviderCardActions } from "@/components/providers/ProviderCardActions";

describe("ProviderCardActions — 统一供应商的编辑入口", () => {
  it("onEdit 存在时菜单里出现「编辑统一供应商」，并回调", async () => {
    const onEdit = vi.fn();
    const user = userEvent.setup();
    render(
      <ProviderCardActions
        providerName="s2OpenAI"
        presentation={{ buttons: [], chips: [] }}
        onEdit={onEdit}
        onDelete={vi.fn()}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "providerCard.action.more" }),
    );
    const menu = await screen.findByRole("menu");
    const item = within(menu).getByRole("menuitem", {
      name: "编辑统一供应商",
    });
    await user.click(item);
    expect(onEdit).toHaveBeenCalledTimes(1);
  });

  it("没有 onEdit 时（普通供应商）不出现这个入口", async () => {
    const user = userEvent.setup();
    render(
      <ProviderCardActions
        providerName="OpenAI Official"
        presentation={{ buttons: [], chips: [] }}
        onDelete={vi.fn()}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "providerCard.action.more" }),
    );
    const menu = await screen.findByRole("menu");
    expect(
      within(menu).queryByRole("menuitem", { name: "编辑统一供应商" }),
    ).toBeNull();
  });
});
