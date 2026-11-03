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
