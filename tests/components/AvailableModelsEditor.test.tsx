import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import { beforeAll } from "vitest";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AvailableModelsEditor } from "@/components/universal/AvailableModelsEditor";
import type { AvailableModel } from "@/types";

beforeAll(() => {
  // Radix 的 Select/Popover 选项在 jsdom 里要求 scrollIntoView
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: vi.fn(),
  });
});

const models: AvailableModel[] = [
  { id: "gpt-5.6-sol", displayName: "GPT-5.6 Sol", contextWindow: 1000000 },
  { id: "glm-5.3" },
];

const setup = (
  over: Partial<React.ComponentProps<typeof AvailableModelsEditor>> = {},
) => {
  const onChange = vi.fn();
  const onRefresh = vi.fn();
  render(
    <AvailableModelsEditor
      models={models}
      onChange={onChange}
      onRefresh={onRefresh}
      {...over}
    />,
  );
  return { onChange, onRefresh };
};

/** 列表默认收起：编辑行之前先点标题展开。 */
const expand = async (user: ReturnType<typeof userEvent.setup>) => {
  await user.click(screen.getByRole("button", { name: /可用模型列表/ }));
};

describe("AvailableModelsEditor", () => {
  it("把列表显示成可编辑的行", async () => {
    const user = userEvent.setup();
    setup();
    // 默认收起：只有标题行和计数，行不在 DOM 里（count 参数会插值）
    expect(screen.getByText("2 个")).toBeInTheDocument();
    expect(screen.queryByDisplayValue("gpt-5.6-sol")).toBeNull();
    await expand(user);
    expect(screen.getByDisplayValue("gpt-5.6-sol")).toBeInTheDocument();
    expect(screen.getByDisplayValue("GPT-5.6 Sol")).toBeInTheDocument();
    expect(screen.getByDisplayValue("1000000")).toBeInTheDocument();
  });

  it("改显示名会回传改过的列表", async () => {
    const user = userEvent.setup();
    const seen: AvailableModel[][] = [];
    // 受控组件：真实用法里父级会把新列表传回来，这里同样用一个有状态的包装
    const Wrapper = () => {
      const [list, setList] = useState<AvailableModel[]>(models);
      return (
        <AvailableModelsEditor
          models={list}
          onChange={(next) => {
            seen.push(next);
            setList(next);
          }}
          onRefresh={vi.fn()}
        />
      );
    };
    render(<Wrapper />);
    await expand(user);
    const name = screen.getByDisplayValue("GPT-5.6 Sol");
    await user.clear(name);
    await user.type(name, "Sol");
    const last = seen.at(-1)!;
    expect(last[0].displayName).toBe("Sol");
    // 其他行不受影响
    expect(last[1]).toEqual({ id: "glm-5.3" });
  });

  it("删除一行", async () => {
    const user = userEvent.setup();
    const { onChange } = setup();
    await expand(user);
    await user.click(screen.getByRole("button", { name: "删除 gpt-5.6-sol" }));
    expect(onChange).toHaveBeenCalledWith([{ id: "glm-5.3" }]);
  });

  it("手动添加一行空行", async () => {
    const user = userEvent.setup();
    const { onChange } = setup();
    await expand(user);
    await user.click(screen.getByRole("button", { name: /手动添加/ }));
    const last = onChange.mock.calls.at(-1)![0] as AvailableModel[];
    expect(last).toHaveLength(3);
    expect(last[2].id).toBe("");
  });

  it("清空 id 等于删掉这一行", async () => {
    const user = userEvent.setup();
    const { onChange } = setup();
    await expand(user);
    const idInput = screen.getByDisplayValue("glm-5.3");
    await user.clear(idInput);
    await user.tab();
    expect(onChange).toHaveBeenCalledWith([
      { id: "gpt-5.6-sol", displayName: "GPT-5.6 Sol", contextWindow: 1000000 },
    ]);
  });

  it("改过之后可以恢复到上次拉取的列表", async () => {
    const user = userEvent.setup();
    const fetched: AvailableModel[] = [{ id: "fetched-only" }];
    const { onChange } = setup({ fetched });
    await user.click(
      screen.getByRole("button", { name: /恢复为上次拉取的结果/ }),
    );
    expect(onChange).toHaveBeenCalledWith(fetched);
  });

  it("没有拉取过时不显示恢复按钮", () => {
    setup();
    expect(screen.queryByRole("button", { name: /恢复/ })).toBeNull();
  });

  it("新增的空行不参与 save 时的落库（由表单过滤）", async () => {
    // 这里只保证空行的 id 确实为空，表单保存时会把它剔掉
    const user = userEvent.setup();
    const { onChange } = setup();
    await expand(user);
    await user.click(screen.getByRole("button", { name: /手动添加/ }));
    const last = onChange.mock.calls.at(-1)![0] as AvailableModel[];
    expect(last.at(-1)!.id).toBe("");
  });

  it("空列表展开给出提示，拉取按钮收起时也可用", async () => {
    const user = userEvent.setup();
    setup({ models: [] });
    // 收起时拉取按钮就在标题行上
    expect(screen.getByRole("button", { name: /获取模型列表/ })).toBeEnabled();
    await expand(user);
    expect(screen.getByText(/还没有模型/)).toBeInTheDocument();
  });
});

describe("AvailableModelsEditor — 逐模型思考等级", () => {
  it("每行有思考等级编辑器；拉取补全的档位显示为标签", async () => {
    const user = userEvent.setup();
    const fetched = [
      { id: "gpt-5.6-sol", efforts: ["low", "high", "max"] },
      { id: "plain" },
    ];
    render(
      <AvailableModelsEditor
        models={fetched}
        onChange={vi.fn()}
        onRefresh={vi.fn()}
      />,
    );
    await expand(user);
    await waitFor(() =>
      expect(
        screen
          .queryAllByRole("combobox")
          .some((combo) => combo.textContent?.includes("low, high, max")),
      ).toBe(true),
    );
    // gpt-5.6-sol 行：触发器列出全部档位（low/high/max 不相邻不压缩）
    const effortTrigger = screen
      .queryAllByRole("combobox")
      .find((combo) => combo.textContent?.includes("low, high, max"))!;
    expect(effortTrigger).toHaveTextContent("low, high, max");
    // plain 行：未声明 → 未设置
    expect(screen.getAllByText("未设置").length).toBeGreaterThan(0);
  });

  it("选择档位后经 patch 写回该行", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const fetched = [{ id: "gpt-5.6-sol", efforts: ["low", "high", "max"] }];
    const Wrapper = () => {
      const [list, setList] = useState<AvailableModel[]>(fetched);
      return (
        <AvailableModelsEditor
          models={list}
          onChange={(next) => {
            onChange(next);
            setList(next);
          }}
          onRefresh={vi.fn()}
        />
      );
    };
    render(<Wrapper />);
    await expand(user);
    await waitFor(() =>
      expect(
        screen
          .queryAllByRole("combobox")
          .some((combo) => combo.textContent?.includes("low, high, max")),
      ).toBe(true),
    );
    await user.click(
      screen
        .queryAllByRole("combobox")
        .find((combo) => combo.textContent?.includes("low, high, max"))!,
    );
    // 弹层里勾选 none（当前未选）
    const dialog = screen.getByRole("dialog");
    const noneItem = within(dialog).getAllByText("none").at(-1)!;
    await user.click(noneItem);
    // 勾选顺序无关：存的是预置升序（none 在最前）
    const last = onChange.mock.calls.at(-1)![0] as AvailableModel[];
    expect(last[0].efforts).toEqual(["none", "low", "high", "max"]);
  });

  it("声明顺序乱掉的档位按预置升序显示", async () => {
    // 拉取的档位来自 oh-my-pi 模型目录的声明顺序（同名模型跨供应商并集保持首现序），
    // 可能是乱的。触发器标签必须按预置序重排，不是照抄输入顺序。
    const user = userEvent.setup();
    render(
      <AvailableModelsEditor
        models={[{ id: "kimi-k2.6", efforts: ["max", "high", "low"] }]}
        onChange={vi.fn()}
        onRefresh={vi.fn()}
      />,
    );
    await expand(user);
    const trigger = screen
      .getAllByRole("combobox")
      .find((combo) => combo.textContent?.includes("max"))!;
    // low < medium < high：三个不相邻预置档位不压缩成范围，逐项列出且按预置序
    expect(trigger).toHaveTextContent("low, high, max");
  });
});

describe("AvailableModelsEditor — 快速配置", () => {
  it("没传 onQuickApply 时不显示快速配置行", () => {
    setup();
    expect(screen.queryByText(/快速配置/)).toBeNull();
  });

  it("选了模型点应用，把模型 id 回传给表单", async () => {
    const user = userEvent.setup();
    const onQuickApply = vi.fn();
    setup({ onQuickApply, desktopMode: "proxy" });
    await expand(user);
    // 打开快速配置的模型下拉，选 glm-5.3
    await user.click(
      screen.getByRole("combobox", { name: /选择要配置的模型/ }),
    );
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByText("glm-5.3"));
    await user.click(screen.getByRole("button", { name: /应用到所有应用/ }));
    // API 类型没选时第三个参数是 undefined（不设置 = 不动任何应用的格式字段）
    expect(onQuickApply).toHaveBeenCalledWith("glm-5.3", undefined, undefined);
  });

  it("选了 API 类型后随应用一起回传", async () => {
    const user = userEvent.setup();
    const onQuickApply = vi.fn();
    setup({ onQuickApply, desktopMode: "proxy" });
    await expand(user);
    await user.click(
      screen.getByRole("combobox", { name: /选择要配置的模型/ }),
    );
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByText("glm-5.3"));
    // 选 API 类型：Anthropic Messages（Select 渲染成 listbox 而不是 dialog）
    await user.click(screen.getByRole("combobox", { name: /API 类型/ }));
    const apiListbox = screen.getByRole("listbox");
    await user.click(within(apiListbox).getByText("Anthropic Messages"));
    await user.click(screen.getByRole("button", { name: /应用到所有应用/ }));
    expect(onQuickApply).toHaveBeenCalledWith(
      "glm-5.3",
      undefined,
      "anthropic",
    );
  });

  it("没选模型时应用按钮不可用", () => {
    setup({ onQuickApply: vi.fn(), desktopMode: "proxy" });
    expect(
      screen.getByRole("button", { name: /应用到所有应用/ }),
    ).toBeDisabled();
  });
});
