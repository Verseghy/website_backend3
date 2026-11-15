import { describe, expect, it } from "vitest"

import { attributeOf, element, text, treeFromDom } from "./stored-tree"

describe("treeFromDom", () => {
  it("copies elements, attributes in order and text", () => {
    const container = document.createElement("div")
    container.innerHTML =
      '<p style="text-align: center" data-x="1">a <strong>b</strong></p><hr>'
    expect(treeFromDom(container.childNodes)).toEqual([
      element(
        "p",
        [
          ["style", "text-align: center"],
          ["data-x", "1"],
        ],
        [text("a "), element("strong", [], [text("b")])]
      ),
      element("hr"),
    ])
  })

  it("reads the style attribute as written, never through the CSSOM", () => {
    const span = document.createElement("span")
    span.setAttribute("style", "color: #c00000")
    // jsdom's CSSOM re-spells it.
    expect(span.style.cssText).toBe("color: rgb(192, 0, 0);")
    expect(treeFromDom([span])).toEqual([
      element("span", [["style", "color: #c00000"]]),
    ])
  })

  it("takes a fragment's children in its place and leaves comments out", () => {
    const fragment = document.createDocumentFragment()
    fragment.append(
      document.createComment("x"),
      document.createTextNode("a"),
      document.createElement("br")
    )
    expect(treeFromDom([fragment])).toEqual([text("a"), element("br")])
  })

  it("keeps an empty text node and an empty attribute", () => {
    const p = document.createElement("p")
    p.setAttribute("data-youtube-video", "")
    p.append(document.createTextNode(""))
    expect(treeFromDom([p])).toEqual([
      element("p", [["data-youtube-video", ""]], [text("")]),
    ])
  })
})

describe("attributeOf", () => {
  it("reads an attribute, or null when it is missing", () => {
    const node = element("a", [
      ["href", "/x"],
      ["rel", ""],
    ])
    expect(attributeOf(node, "href")).toBe("/x")
    expect(attributeOf(node, "rel")).toBe("")
    expect(attributeOf(node, "target")).toBeNull()
  })
})
