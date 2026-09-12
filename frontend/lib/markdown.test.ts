// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { renderMessageMarkdown } from "./markdown";

function render(message: string) {
  const root = document.createElement("div");
  root.innerHTML = renderMessageMarkdown(message);
  return root;
}

describe("task message Markdown", () => {
  it("renders headings, emphasis, lists, code, links and tables", () => {
    const root = render('# 结果\n\n**完成** `git status`\n\n- 第一项\n- 第二项\n\n```sh\necho "<ok>"\n```\n\n[文档](https://example.com)\n\n| 项目 | 状态 |\n| --- | --- |\n| 测试 | 通过 |');
    expect(root.querySelector("h1")?.textContent).toBe("结果");
    expect(root.querySelector("strong")?.textContent).toBe("完成");
    expect(root.querySelectorAll("li")).toHaveLength(2);
    expect(root.querySelector("pre code")?.textContent).toContain('<ok>');
    expect(root.querySelector("a")?.getAttribute("href")).toBe("https://example.com");
    expect(root.querySelectorAll("td")).toHaveLength(2);
  });

  it("keeps plain text and decoded entities readable and preserves code literally", () => {
    const root = render('普通消息 &#x20; &amp; 内容\n换行\n\n`<script>alert(1)</script>`');
    expect(root.textContent).toContain("普通消息   & 内容");
    expect(root.querySelector("br")).not.toBeNull();
    expect(root.querySelector("code")?.textContent).toBe("<script>alert(1)</script>");
    expect(root.querySelector("script")).toBeNull();
  });

  it("removes executable HTML, embedded resources and unsafe URLs", () => {
    const root = render('<script>alert(1)</script>\n\n<img src=x onerror=alert(1)><iframe src="https://example.com"></iframe><svg onload="alert(1)"></svg>\n\n[x](javascript:alert%281%29) [file](file:///etc/passwd) [data](data:text/html,test)\n\n<a href="jav&#x61;script:alert(1)" onclick="alert(1)">bad</a><p style="position:fixed" id="pet">text</p>');
    expect(root.querySelector("script,img,iframe,svg,[onclick],[onerror],[style],[id]")).toBeNull();
    expect(root.querySelectorAll("a[href]")).toHaveLength(0);
  });

  it("renders task lists and image descriptions without live controls or remote loads", () => {
    const root = render('- [x] 完成\n- [ ] 待办\n\n![结果预览](https://example.com/image.png)');
    expect(root.textContent).toContain("☑ 完成");
    expect(root.textContent).toContain("☐ 待办");
    expect(root.textContent).toContain("结果预览");
    expect(root.querySelector("input,img")).toBeNull();
  });
});
