import { Editor } from "@tiptap/core"
import { describe, expect, it } from "vitest"

import type { SiteImageAttributes } from "../image-attributes"
import { loadDocument } from "../load-document"
import { CONTENT_V1_POLICY, PolicyError } from "../policy"
import { toStoredHtml } from "../serialize/to-stored-html"
import { siteExtensions } from "../site-extensions"
import { createSiteImage, SITE_IMAGE_NODE, wrappedImage } from "./site-image"
import { policyWith } from "@/test/rich-text-policy"
import { parseHtml, siteSchema } from "@/test/site-schema"
import { storedHtmlHarness } from "@/test/stored-html"

const LOCKED = "display: block; height: auto; margin: 0 auto; max-width: 100%"

/** The stored image shape of editor.md §3.7. */
function stored(attributes: string, width?: number) {
  const style = width === undefined ? LOCKED : `${LOCKED}; width: ${width}px`
  return `<p><img ${attributes} style="${style}"></p>`
}

const SHAPE = stored(
  'src="/storage/uploads/posts/612/kep.jpg" alt="A díjazottak a színpadon" width="350" height="495"',
  350
)

describe("the image's stored shape (editor.md §3.7)", () => {
  const harness = storedHtmlHarness()

  it.each([
    SHAPE,
    stored('src="/storage/uploads/a.png" alt="" width="700" height="350"', 700),
    stored('src="/storage/uploads/a.png" alt="kép"'),
    stored('src="/storage/uploads/a.png" width="175"', 175),
    stored(
      'src="/storage/uploads/a.png" alt="széles" width="1200" height="600"',
      1200
    ),
    stored(
      'src="/storage/uploads/a.png" alt="tört" width="351" height="496"',
      350.5
    ),
  ])("is a fixed point: %s", (body) => {
    expect(harness.J(body)).toBe(body)
  })

  it("is written in the canonical form from legacy sizes, then stays", () => {
    for (const [input, output] of [
      // The style's width wins; the height follows the ratio of the
      // attributes.
      [
        '<p><img src="/storage/uploads/a.jpg" alt="a" width="700" height="350" style="width: 350px"></p>',
        stored(
          'src="/storage/uploads/a.jpg" alt="a" width="350" height="175"',
          350
        ),
      ],
      // The width attribute alone gives the width, within the style's range.
      [
        '<p><img src="/storage/uploads/a.jpg" alt="a" width="3000" height="1500"></p>',
        stored(
          'src="/storage/uploads/a.jpg" alt="a" width="2000" height="1000"',
          2000
        ),
      ],
      // A legacy pixel height becomes auto, as the server stores it.
      [
        '<p><img src="https://verseghy-gimnazium.net/storage/uploads/a.jpg" alt="a" style="height: 495px; width: 262.5pt"></p>',
        stored('src="/storage/uploads/a.jpg" alt="a" width="350"', 350),
      ],
    ]) {
      expect(harness.J(input)).toBe(output)
      expect(harness.J(output)).toBe(output)
    }
  })
})

describe("the image parse rules", () => {
  const schema = siteSchema()
  const imagesOf = (html: string) => {
    const doc = parseHtml(schema, html)
    const found: { attrs: SiteImageAttributes; topLevel: boolean }[] = []
    doc.descendants((node, _position, parent) => {
      if (node.type.name === SITE_IMAGE_NODE) {
        found.push({
          attrs: node.attrs as SiteImageAttributes,
          topLevel: parent === doc,
        })
      }
      return true
    })
    return found
  }

  it("reads the stored shape (rule ①)", () => {
    expect(imagesOf(SHAPE)).toEqual([
      {
        attrs: {
          src: "/storage/uploads/posts/612/kep.jpg",
          alt: "A díjazottak a színpadon",
          displayWidth: 350,
          ratio: 495 / 350,
          uploadState: null,
          localKey: null,
        },
        topLevel: true,
      },
    ])
  })

  it.each([
    '<p><a href="https://example.org/"><img src="/storage/uploads/a.jpg" alt="a"></a></p>',
    '<p><strong><em><img src="/storage/uploads/a.jpg" alt="a"></em></strong></p>',
    '<p>\n <span><img src="/storage/uploads/a.jpg" alt="a"></span>\n</p>',
    '<img src="/storage/uploads/a.jpg" alt="a">',
  ])("reads the image of %s, dropping its wrappers", (html) => {
    const [image] = imagesOf(html)
    expect(image.attrs.src).toBe("/storage/uploads/a.jpg")
    expect(image.topLevel).toBe(true)
    const doc = parseHtml(schema, html)
    let links = 0
    doc.descendants((node) => {
      links += node.marks.filter((mark) => mark.type.name === "link").length
      return true
    })
    expect(links).toBe(0)
  })

  it("moves an image out of a cell, list or quote (rule ②, a fallback)", () => {
    for (const html of [
      '<table><tr><td><img src="/storage/uploads/a.jpg" alt="a"></td></tr></table>',
      '<ul><li>pont <img src="/storage/uploads/a.jpg" alt="a"></li></ul>',
      '<blockquote><img src="/storage/uploads/a.jpg" alt="a"></blockquote>',
    ]) {
      expect(imagesOf(html).map(({ topLevel }) => topLevel)).toEqual([true])
    }
  })

  it.each([
    '<p><img src="https://evil.example/a.jpg" alt="a"></p>',
    '<p><img src="data:image/png;base64,AA" alt="a"></p>',
    '<p><img src="/storage/uploads/a.svg" alt="a"></p>',
    '<p><img alt="nincs forrás"></p>',
  ])("loads no image the server would refuse: %s", (html) => {
    expect(imagesOf(html)).toEqual([])
  })

  it("keeps a paragraph with text and an image as text and an image", () => {
    const doc = parseHtml(
      schema,
      '<p>előtte <img src="/storage/uploads/a.jpg" alt="a"> utána</p>'
    )
    const names: string[] = []
    doc.forEach((node) => names.push(node.type.name))
    expect(names).toEqual(["paragraph", SITE_IMAGE_NODE, "paragraph"])
  })
})

describe("wrappedImage", () => {
  const paragraph = (html: string) => {
    const container = document.createElement("div")
    container.innerHTML = html
    return container.firstElementChild!
  }

  it("finds an image alone or in a chain of a, strong, em and span", () => {
    expect(wrappedImage(paragraph("<p><img src=a></p>"))).not.toBeNull()
    expect(
      wrappedImage(paragraph("<p><a><span><img src=a></span></a></p>"))
    ).not.toBeNull()
    expect(wrappedImage(paragraph("<p><u><img src=a></u></p>"))).toBeNull()
    expect(wrappedImage(paragraph("<p><img></p>"))).toBeNull()
    expect(wrappedImage(paragraph("<p>x<img src=a></p>"))).toBeNull()
  })
})

describe("the image render rule", () => {
  const schema = siteSchema()
  const render = (attrs: Partial<SiteImageAttributes>) =>
    toStoredHtml(
      schema.nodes.doc.create(null, [
        schema.nodes[SITE_IMAGE_NODE].create({
          src: "/storage/uploads/a.jpg",
          ...attrs,
        }),
      ]),
      { policy: CONTENT_V1_POLICY, document }
    )

  it("never stores the upload state or the local key", () => {
    expect(render({ alt: "a", uploadState: "pending", localKey: "k1" })).toBe(
      stored('src="/storage/uploads/a.jpg" alt="a"')
    )
  })

  it("writes no value the server would drop", () => {
    expect(render({ src: "blob:x", displayWidth: 2001, ratio: 1 })).toBe(
      stored("").replace("<img ", "<img")
    )
  })
})

describe("insertSiteImage", () => {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
  })
  const save = () =>
    toStoredHtml(editor.state.doc, { policy: CONTENT_V1_POLICY, document })

  it("inserts at the natural width, at most the column's 700 px", () => {
    loadDocument(editor, "<p></p>")
    expect(
      editor.commands.insertSiteImage({
        src: "/storage/uploads/a.jpg",
        alt: "a",
        width: 1400,
        height: 700,
      })
    ).toBe(true)
    expect(save()).toBe(
      stored(
        'src="/storage/uploads/a.jpg" alt="a" width="700" height="350"',
        700
      )
    )
    loadDocument(editor, "<p></p>")
    editor.commands.insertSiteImage({
      src: "/storage/uploads/b.png",
      alt: null,
      width: 300,
      height: 200,
    })
    expect(save()).toBe(
      stored('src="/storage/uploads/b.png" width="300" height="200"', 300)
    )
  })

  it.each([
    { src: "https://evil.example/a.jpg", alt: "a", width: 10, height: 10 },
    { src: "/storage/uploads/a.jpg", alt: "a", width: 0, height: 10 },
    { src: "/storage/uploads/a.jpg", alt: "a", width: 10, height: 1.5 },
  ])("refuses %j", (image) => {
    loadDocument(editor, "<p>x</p>")
    expect(editor.commands.insertSiteImage(image)).toBe(false)
    expect(save()).toBe("<p>x</p>")
  })
})

describe("createSiteImage", () => {
  it("refuses a policy without the image's width grammar", () => {
    const policy = policyWith((json) => {
      const elements = json.elements as {
        img: { style: { property: string }[] }
      }
      elements.img.style = elements.img.style.filter(
        (rule) => rule.property !== "width"
      )
    })
    expect(() => createSiteImage(policy)).toThrow(PolicyError)
  })
})
