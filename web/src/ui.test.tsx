import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Avatar, Confirm, CustomFieldInput, Dialog, Markdown, Priority, SlaChip, StatusBadge, fieldText } from "./ui";
import type { FieldDef, Person } from "./api";

const people: Person[] = [
  { id: "u1", name: "Ada Okafor", kind: "human" } as Person,
  { id: "a1", name: "Triage agent", kind: "agent" } as Person,
];
const field = (type: string, options: string[] = []) => ({ key: "f", name: "Field", type, options, description: "" }) as unknown as FieldDef;

describe("status and priority", () => {
  it("shows a status by name and marks its category for color", () => {
    const { container } = render(<StatusBadge name="In review" category="in_progress" />);
    expect(screen.getByText("In review")).toBeInTheDocument();
    expect(container.querySelector(".badge")).toHaveClass("in_progress");
  });
  it("always says the priority in words, even when only the icon shows", () => {
    render(<Priority value={4} label={false} />);
    expect(screen.getByText("Critical")).toHaveClass("sr-only");
    expect(screen.getByTitle("Priority: Critical")).toBeInTheDocument();
  });
  it("draws people and agents differently and labels an empty seat", () => {
    render(<><Avatar person={people[0]} /><Avatar person={people[1]} /><Avatar person={null} /></>);
    expect(screen.getByTitle("Ada Okafor")).toHaveTextContent("AO");
    expect(screen.getByTitle("Triage agent (AI agent)")).toHaveClass("agent");
    expect(screen.getByLabelText("Unassigned")).toBeInTheDocument();
  });
  it("shows nothing for a request without a target and a state for one with", () => {
    const { container, rerender } = render(<SlaChip due={null} breached={false} metAt={null} what="Resolution" />);
    expect(container).toBeEmptyDOMElement();
    rerender(<SlaChip due="2020-01-01T00:00:00Z" breached={true} metAt={null} what="Resolution" />);
    expect(container.querySelector(".sla")).toHaveClass("late");
    rerender(<SlaChip due="2020-01-01T00:00:00Z" breached={false} metAt="2019-12-31T00:00:00Z" what="Resolution" />);
    expect(screen.getByText("Met")).toBeInTheDocument();
  });
});

describe("dialogs", () => {
  it("is a labeled modal that focuses its first field and closes on Escape", () => {
    const onClose = vi.fn();
    render(
      <Dialog title="New item" onClose={onClose}>
        <input aria-label="Title" />
      </Dialog>,
    );
    const dialog = screen.getByRole("dialog", { name: "New item" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(screen.getByLabelText("Title")).toHaveFocus();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  it("keeps Tab among its own controls, and leaves keys to the dialog on top", () => {
    // jsdom lays nothing out, so every element reports no boxes; say they are all on screen.
    const rects = vi.spyOn(HTMLElement.prototype, "getClientRects").mockReturnValue([{}] as unknown as DOMRectList);
    const closeUnder = vi.fn(), closeTop = vi.fn();
    render(
      <>
        <button>Behind the dialog</button>
        <Dialog title="Under" onClose={closeUnder}>
          <input aria-label="Under field" />
        </Dialog>
        <Dialog title="Top" onClose={closeTop} footer={<button className="btn primary">Save</button>}>
          <input aria-label="First" />
        </Dialog>
      </>,
    );
    const top = screen.getByRole("dialog", { name: "Top" });
    const save = screen.getByRole("button", { name: "Save" });
    const close = top.querySelector<HTMLElement>('[aria-label="Close"]')!;
    // Forward from the last control wraps to the first; backward from the first wraps to the last.
    save.focus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(close).toHaveFocus();
    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(save).toHaveFocus();
    // Focus that ended up outside comes back in on the next Tab.
    screen.getByRole("button", { name: "Behind the dialog" }).focus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(top).toContainElement(document.activeElement as HTMLElement);
    // Escape closes only the dialog on top.
    fireEvent.keyDown(document, { key: "Escape" });
    expect(closeTop).toHaveBeenCalledTimes(1);
    expect(closeUnder).not.toHaveBeenCalled();
    rects.mockRestore();
  });
  it("closes from the close button and from a click outside, not from a click inside", () => {
    const onClose = vi.fn();
    const { container } = render(
      <Dialog title="T" onClose={onClose}>
        <p>Body</p>
      </Dialog>,
    );
    fireEvent.mouseDown(screen.getByText("Body"));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.click(screen.getByLabelText("Close"));
    fireEvent.mouseDown(container.querySelector(".backdrop")!);
    expect(onClose).toHaveBeenCalledTimes(2);
  });
  it("runs a confirmed action once and only when confirmed", async () => {
    const onConfirm = vi.fn();
    const onClose = vi.fn();
    render(<Confirm title="Archive item?" body="It can be restored." action="Archive" danger onConfirm={onConfirm} onClose={onClose} />);
    fireEvent.click(screen.getByText("Cancel"));
    expect(onConfirm).not.toHaveBeenCalled();
    const go = screen.getByRole("button", { name: "Archive" });
    expect(go).toHaveClass("danger");
    fireEvent.click(go);
    await vi.waitFor(() => expect(onClose).toHaveBeenCalledTimes(2));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});

describe("custom fields", () => {
  it("reports numbers as numbers and an emptied field as no value", () => {
    const onChange = vi.fn();
    const { container } = render(<CustomFieldInput field={field("number")} value={3} onChange={onChange} people={people} />);
    const input = container.querySelector("input")!;
    fireEvent.change(input, { target: { value: "4.5" } });
    fireEvent.change(input, { target: { value: "" } });
    expect(onChange.mock.calls).toEqual([[4.5], [null]]);
  });
  it("offers only the configured choices", () => {
    const onChange = vi.fn();
    render(<CustomFieldInput field={field("select", ["Low", "High"])} value="Low" onChange={onChange} people={people} />);
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["Not set", "Low", "High"]);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "High" } });
    expect(onChange).toHaveBeenCalledWith("High");
  });
  it("adds and removes choices in a multiple-choice field", () => {
    const onChange = vi.fn();
    render(<CustomFieldInput field={field("multiselect", ["A", "B"])} value={["A"]} onChange={onChange} people={people} />);
    fireEvent.click(screen.getByLabelText("B"));
    fireEvent.click(screen.getByLabelText("A"));
    expect(onChange.mock.calls).toEqual([[["A", "B"]], [[]]]);
  });
  it("describes stored values in words", () => {
    expect(fieldText(field("checkbox"), false, people)).toBe("No");
    expect(fieldText(field("checkbox"), true, people)).toBe("Yes");
    expect(fieldText(field("user"), "u1", people)).toBe("Ada Okafor");
    expect(fieldText(field("user"), "gone", people)).toBe("Unknown person");
    expect(fieldText(field("multiselect"), ["A", "B"], people)).toBe("A, B");
    expect(fieldText(field("number"), 0, people)).toBe("0");
    expect(fieldText(field("text"), null, people)).toBe("");
  });
});

describe("Markdown component", () => {
  it("never lets content run script or carry handlers into the page", () => {
    const { container } = render(<Markdown source={'Hello <img src=x onerror="window.pwned=1"> [x](javascript:window.pwned=1)\n\n<script>window.pwned=1</script>'} />);
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("[onerror]")).toBeNull();
    expect(container.innerHTML).not.toMatch(/href="javascript:/i);
    expect((window as any).pwned).toBeUndefined();
  });
  it("shows a placeholder for empty text", () => {
    render(<Markdown source="" empty="No description yet." />);
    expect(screen.getByText("No description yet.")).toBeInTheDocument();
  });
});
