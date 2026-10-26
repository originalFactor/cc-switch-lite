import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect, vi, beforeEach } from "vitest";
import type { ReactElement } from "react";
import type { Provider } from "@/types";
import {
  ProviderList,
  type SwitchModeProps,
} from "@/components/providers/ProviderList";

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  },
}));

const useDragSortMock = vi.fn();
const useSortableMock = vi.fn();
const providerCardRenderSpy = vi.fn();
/** 某张卡片最近一次渲染拿到的 props。 */
const lastProps = (id: string) =>
  providerCardRenderSpy.mock.calls
    .map((call) => call[0])
    .filter((props) => props.provider.id === id)
    .at(-1);

vi.mock("@/hooks/useDragSort", () => ({
  useDragSort: (...args: unknown[]) => useDragSortMock(...args),
}));

vi.mock("@/components/providers/ProviderCard", () => ({
  ProviderCard: (props: any) => {
    providerCardRenderSpy(props);
    const { provider, presentation, onDelete } = props;

    return (
      <div data-testid={`provider-card-${provider.id}`}>
        {presentation.status && (
          <span data-testid={`status-${provider.id}`}>
            {presentation.status.label}
          </span>
        )}
        {presentation.buttons.map((button: any) => (
          <button
            key={button.key}
            data-testid={`${button.key}-${provider.id}`}
            disabled={Boolean(button.disabledReason)}
            onClick={button.onClick}
          >
            {button.label}
          </button>
        ))}
        <button
          data-testid={`delete-${provider.id}`}
          onClick={() => onDelete(provider)}
        >
          delete
        </button>
        <span data-testid={`drag-attr-${provider.id}`}>
          {props.dragHandleProps?.attributes?.["data-dnd-id"] ?? "none"}
        </span>
      </div>
    );
  },
}));

vi.mock("@/components/UsageFooter", () => ({
  default: () => <div data-testid="usage-footer" />,
}));

vi.mock("@dnd-kit/sortable", async () => {
  const actual = await vi.importActual<any>("@dnd-kit/sortable");

  return {
    ...actual,
    useSortable: (...args: unknown[]) => useSortableMock(...args),
  };
});

// Mock hooks that use QueryClient
vi.mock("@/hooks/useStreamCheck", () => ({
  useStreamCheck: () => ({
    checkProvider: vi.fn(),
    isChecking: () => false,
  }),
}));

function createProvider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: overrides.id ?? "provider-1",
    name: overrides.name ?? "Test Provider",
    settingsConfig: overrides.settingsConfig ?? {},
    category: overrides.category,
    createdAt: overrides.createdAt,
    sortIndex: overrides.sortIndex,
    meta: overrides.meta,
    websiteUrl: overrides.websiteUrl,
  };
}

function renderWithQueryClient(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

beforeEach(() => {
  useDragSortMock.mockReset();
  useSortableMock.mockReset();
  providerCardRenderSpy.mockClear();

  useSortableMock.mockImplementation(({ id }: { id: string }) => ({
    setNodeRef: vi.fn(),
    attributes: { "data-dnd-id": id },
    listeners: { onPointerDown: vi.fn() },
    transform: null,
    transition: null,
    isDragging: false,
  }));

  useDragSortMock.mockReturnValue({
    sortedProviders: [],
    sensors: [],
    handleDragEnd: vi.fn(),
  });
});

describe("ProviderList Component", () => {
  const baseProps = {
    currentProviderId: "",
    onSwitch: vi.fn(),
    onDelete: vi.fn(),
    onOpenWebsite: vi.fn(),
  };

  function switchMode(
    overrides: Partial<SwitchModeProps> = {},
  ): SwitchModeProps {
    return {
      active: "direct",
      directId: null,
      routeId: null,
      failoverOn: false,
      queue: [],
      routingReason: () => "",
      serviceRunning: false,
      actions: {
        switchDirect: vi.fn(),
        needsRouteDialog: vi.fn(),
          routeTo: vi.fn(),
        queueAdd: vi.fn(),
        queueRemove: vi.fn(),
        queueMove: vi.fn(),
      },
      ...overrides,
    };
  }

  it("should render skeleton placeholders when loading", () => {
    const { container } = renderWithQueryClient(
      <ProviderList {...baseProps} providers={{}} appId="claude" isLoading />,
    );

    expect(container.querySelectorAll(".border-dashed")).toHaveLength(3);
  });

  it("should show empty state and trigger create callback when no providers exist", () => {
    const handleCreate = vi.fn();

    renderWithQueryClient(
      <ProviderList
        {...baseProps}
        providers={{}}
        appId="claude"
        onCreate={handleCreate}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "provider.noProviders" }),
    ).toBeInTheDocument();
    // 页头已经有实心的「添加供应商」，空状态里的两个按钮都是描边
    const importButton = screen.getByRole("button", {
      name: "provider.importCurrent",
    });
    const addButton = screen.getByRole("button", {
      name: "provider.addProvider",
    });
    for (const button of [importButton, addButton]) {
      expect(button.className).toContain("border-border-strong");
      expect(button.className).not.toContain("bg-action");
    }

    fireEvent.click(addButton);
    expect(handleCreate).toHaveBeenCalledTimes(1);
  });

  it("renders in the order returned by useDragSort and wires the mode actions", () => {
    const providerA = createProvider({ id: "a", name: "A" });
    const providerB = createProvider({ id: "b", name: "B" });
    const handleDelete = vi.fn();
    const mode = switchMode({ directId: "b" });

    useDragSortMock.mockReturnValue({
      sortedProviders: [providerB, providerA],
      sensors: [],
      handleDragEnd: vi.fn(),
    });

    renderWithQueryClient(
      <ProviderList
        {...baseProps}
        providers={{ a: providerA, b: providerB }}
        currentProviderId="b"
        appId="claude"
        onDelete={handleDelete}
        switchMode={mode}
      />,
    );

    expect(providerCardRenderSpy).toHaveBeenCalledTimes(2);
    expect(providerCardRenderSpy.mock.calls[0][0].provider.id).toBe("b");
    expect(providerCardRenderSpy.mock.calls[1][0].provider.id).toBe("a");
    expect(lastProps("b")?.isCurrent).toBe(true);
    expect(screen.getByTestId("drag-attr-b")).toHaveTextContent("b");
    expect(screen.getByTestId("drag-attr-a")).toHaveTextContent("a");

    // 直连那家显示「使用中」，其余是「切换」
    expect(screen.getByTestId("status-b")).toHaveTextContent(
      "providerCard.status.inUse",
    );
    fireEvent.click(screen.getByTestId("switch-a"));
    expect(mode.actions.switchDirect).toHaveBeenCalledWith(providerA);

    fireEvent.click(screen.getByTestId("delete-a"));
    expect(handleDelete).toHaveBeenCalledWith(providerA);
    expect(useDragSortMock).toHaveBeenCalledWith(
      { a: providerA, b: providerB },
      "claude",
    );
  });

  it("shows queue sections while failover is on", () => {
    const providerA = createProvider({ id: "a", name: "A" });
    const providerB = createProvider({ id: "b", name: "B" });
    useDragSortMock.mockReturnValue({
      sortedProviders: [providerA, providerB],
      sensors: [],
      handleDragEnd: vi.fn(),
    });

    renderWithQueryClient(
      <ProviderList
        {...baseProps}
        providers={{ a: providerA, b: providerB }}
        appId="claude"
        switchMode={switchMode({
          active: "route",
          routeId: "b",
          failoverOn: true,
          queue: ["b"],
        })}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "providerCard.section.queue" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("queueRemove-b")).toBeInTheDocument();
    expect(screen.getByTestId("queueAdd-a")).toBeInTheDocument();
  });

  it("filters providers with the search input", () => {
    const providerAlpha = createProvider({ id: "alpha", name: "Alpha Labs" });
    const providerBeta = createProvider({ id: "beta", name: "Beta Works" });

    useDragSortMock.mockReturnValue({
      sortedProviders: [providerAlpha, providerBeta],
      sensors: [],
      handleDragEnd: vi.fn(),
    });

    renderWithQueryClient(
      <ProviderList
        providers={{ alpha: providerAlpha, beta: providerBeta }}
        currentProviderId=""
        appId="claude"
        onSwitch={vi.fn()}
        onDelete={vi.fn()}
        onOpenWebsite={vi.fn()}
      />,
    );

    fireEvent.keyDown(window, { key: "f", metaKey: true });
    const searchInput = screen.getByPlaceholderText(
      "Search name, notes, or URL...",
    );
    // Initially both providers are rendered
    expect(screen.getByTestId("provider-card-alpha")).toBeInTheDocument();
    expect(screen.getByTestId("provider-card-beta")).toBeInTheDocument();

    fireEvent.change(searchInput, { target: { value: "beta" } });
    expect(screen.queryByTestId("provider-card-alpha")).not.toBeInTheDocument();
    expect(screen.getByTestId("provider-card-beta")).toBeInTheDocument();

    fireEvent.change(searchInput, { target: { value: "gamma" } });
    expect(screen.queryByTestId("provider-card-alpha")).not.toBeInTheDocument();
    expect(screen.queryByTestId("provider-card-beta")).not.toBeInTheDocument();
    expect(
      screen.getByText("No providers match your search."),
    ).toBeInTheDocument();
  });

  it("matches the API address and stops filtering once the panel closes", () => {
    const relay = createProvider({
      id: "relay",
      name: "My Relay",
      settingsConfig: {
        env: { ANTHROPIC_BASE_URL: "https://api.relay-example.com" },
      },
    });
    const other = createProvider({ id: "other", name: "Other" });

    useDragSortMock.mockReturnValue({
      sortedProviders: [relay, other],
      sensors: [],
      handleDragEnd: vi.fn(),
    });

    const props = {
      providers: { relay, other },
      currentProviderId: "",
      appId: "claude" as const,
      onSwitch: vi.fn(),
      onDelete: vi.fn(),
      onOpenWebsite: vi.fn(),
    };
    const onSearchOpenChange = vi.fn();
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const ui = (searchOpen: boolean) => (
      <QueryClientProvider client={queryClient}>
        <ProviderList
          {...props}
          searchOpen={searchOpen}
          onSearchOpenChange={onSearchOpenChange}
        />
      </QueryClientProvider>
    );
    const { rerender } = render(ui(true));

    fireEvent.change(
      screen.getByPlaceholderText("Search name, notes, or URL..."),
      { target: { value: "relay-example" } },
    );
    expect(screen.getByTestId("provider-card-relay")).toBeInTheDocument();
    expect(screen.queryByTestId("provider-card-other")).not.toBeInTheDocument();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onSearchOpenChange).toHaveBeenCalledWith(false);

    rerender(ui(false));
    expect(screen.getByTestId("provider-card-other")).toBeInTheDocument();
  });
  it("puts the search field beside the first section title and keeps it while no provider matches", async () => {
    const member = createProvider({ id: "member", name: "Member Labs" });
    const other = createProvider({ id: "other", name: "Other Works" });
    useDragSortMock.mockReturnValue({
      sortedProviders: [member, other],
      sensors: [],
      handleDragEnd: vi.fn(),
    });

    renderWithQueryClient(
      <ProviderList
        {...baseProps}
        providers={{ member, other }}
        appId="claude"
        switchMode={switchMode({
          active: "route",
          routeId: "member",
          failoverOn: true,
          queue: ["member"],
        })}
      />,
    );

    const heading = await screen.findByRole("heading", {
      name: "providerCard.section.queue",
    });
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const input = screen.getByRole("textbox", { name: "Search providers" });
    // 同一行：标题和搜索框共用列表的第一行，第一个分组里不再重复画标题
    const header = input.closest("div.min-h-8");
    expect(header).toContainElement(heading);

    await waitFor(() => expect(input).toHaveFocus());

    fireEvent.change(input, { target: { value: "nothing-matches" } });
    expect(screen.getByText("No providers match your search.")).toBeVisible();
    // 结果清空时输入框还是同一个节点，焦点不丢
    expect(screen.getByRole("textbox", { name: "Search providers" })).toBe(
      input,
    );
    expect(input).toHaveFocus();
  });

  it("adds the search row only while searching when the list has no section title", async () => {
    const desk = createProvider({ id: "desk", name: "Desk Relay" });
    useDragSortMock.mockReturnValue({
      sortedProviders: [desk],
      sensors: [],
      handleDragEnd: vi.fn(),
    });

    const { container } = renderWithQueryClient(
      <ProviderList
        {...baseProps}
        providers={{ desk }}
        appId="claude-desktop"
      />,
    );

    const list = container.firstElementChild!;
    expect(list.firstElementChild).toContainElement(
      screen.getByTestId("provider-card-desk"),
    );

    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const input = screen.getByRole("textbox", { name: "Search providers" });
    expect(list.firstElementChild).toContainElement(input);
    expect(list.firstElementChild?.nextElementSibling).toContainElement(
      screen.getByTestId("provider-card-desk"),
    );

    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(input).not.toBeInTheDocument());
    expect(list.firstElementChild).toContainElement(
      screen.getByTestId("provider-card-desk"),
    );
  });
});
