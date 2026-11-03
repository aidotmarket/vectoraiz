import { afterEach, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { AnswerText } from "./AnswerText";

afterEach(cleanup);
it.each(["**Key", "**Key fa", "_x", "price*", "**", "*", "__", "_", "__Key", "*Key"])("hides only unfinished streaming emphasis in %s", text => {
  const {container, rerender} = render(<AnswerText text={text} streaming />);
  expect(container.textContent).toBe(text.replace(/\*\*|__|\*|_/, ""));
  expect(container.querySelector("strong, em")).toBeNull();
  rerender(<AnswerText text={text} streaming={false} />);
  expect(container.textContent).toBe(text === "**" || text === "__" ? "" : text);
});
it("keeps complete emphasis while hiding the final unfinished opener", () => {
  const {container, rerender} = render(<AnswerText text="**Done** and **Key fa" streaming />);
  expect(container.textContent).toBe("Done and Key fa");
  expect(container.querySelector("strong")?.textContent).toBe("Done");
  rerender(<AnswerText text="**Done** and **Key facts**" streaming />);
  expect(container.querySelectorAll("strong")).toHaveLength(2);
});

it.each(["file_name", "report_2024.pdf", "3*4"])("preserves intraword markers in %s while streaming", identifier => {
  const text = `Use ${identifier} here. **Key facts** are`;
  const {container, rerender} = render(<AnswerText text={text} streaming />);
  expect(container.textContent).toBe(`Use ${identifier} here. Key facts are`);
  expect(container.querySelector("strong")?.textContent).toBe("Key facts");
  const markup = container.innerHTML;
  rerender(<AnswerText text={text} />);
  // Preserve the legacy finished renderer, including its intraword-star quirk.
  expect(container.innerHTML).toBe(identifier === "3*4"
    ? '<p class="whitespace-pre-wrap">Use 3<em>4 here. </em><em>Key facts</em>* are</p>'
    : markup);
});
it.each(["**", "__"])("keeps %s hidden through incremental closing markers", marker => {
  const {container, rerender} = render(<AnswerText text={`${marker}Key`} streaming />);
  expect(container.textContent).toBe("Key");
  rerender(<AnswerText text={`${marker}Key${marker[0]}`} streaming />);
  expect(container.textContent).toBe(`Key${marker[0]}`);
  expect(container.querySelector("strong, em")).toBeNull();
  rerender(<AnswerText text={`${marker}Key${marker}`} streaming />);
  expect(container.textContent).toBe("Key");
  expect(container.querySelector("strong")?.textContent).toBe("Key");
  const markup = container.innerHTML;
  rerender(<AnswerText text={`${marker}Key${marker}`} />);
  expect(container.innerHTML).toBe(markup);
});
