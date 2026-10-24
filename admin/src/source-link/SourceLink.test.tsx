import { screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { renderHungarian } from "@/test/render"

import { SourceLink } from "./SourceLink"

const SHA = "89abcdef0123456789abcdef0123456789abcdef"

describe("SourceLink", () => {
  it("points at the running commit and opens in a new tab", () => {
    renderHungarian(<SourceLink sourceCommit={SHA}>Forráskód</SourceLink>)
    const link = screen.getByRole("link", { name: "Forráskód" })
    expect(link).toHaveAttribute(
      "href",
      `https://github.com/Verseghy/website_backend3/tree/${SHA}`
    )
    expect(link).toHaveAttribute("target", "_blank")
    expect(link).toHaveAttribute("rel", "noopener noreferrer")
    expect(link).toHaveAttribute(
      "title",
      "A futó verzió forráskódja a GitHubon (AGPL-3.0 licenc)"
    )
  })

  it("points at master for a local build", () => {
    renderHungarian(<SourceLink sourceCommit={undefined}>Forráskód</SourceLink>)
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      "https://github.com/Verseghy/website_backend3/tree/master"
    )
  })
})
