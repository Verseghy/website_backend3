import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { DROPPED_OPTIONS } from "../youtube-input"
import {
  emptyReportDraft,
  finishReport,
  YOUTUBE_REFUSALS,
} from "./paste-context"

describe("emptyReportDraft", () => {
  it("starts with nothing dropped or moved, and a new draft each time", () => {
    const draft = emptyReportDraft()
    expect(finishReport(draft)).toEqual({
      moved: { media: [] },
      dropped: {
        hiddenRows: 0,
        hiddenCells: 0,
        hiddenOther: 0,
        fontFamilies: [],
        frames: 0,
        youtubeRefusals: [],
        embedParams: [],
      },
    })
    draft.frames += 1
    draft.fontFamilies.add("Calibri")
    expect(finishReport(emptyReportDraft()).dropped.frames).toBe(0)
    expect(finishReport(emptyReportDraft()).dropped.fontFamilies).toEqual([])
  })
})

describe("finishReport (editor.md §5.2 step 12)", () => {
  it("copies the counts and the moves, and orders the lists", () => {
    const draft = emptyReportDraft()
    draft.hiddenRows = 2
    draft.hiddenCells = 3
    draft.hiddenOther = 1
    draft.frames = 4
    draft.fontFamilies.add("Arial")
    draft.fontFamilies.add("Calibri")
    draft.youtubeRefusals.add("list_type")
    draft.youtubeRefusals.add("video_id")
    draft.embedParams.add("playlist")
    draft.embedParams.add("controls")
    draft.moved.push({ media: "youtube", from: "td" })
    const report = finishReport(draft)
    expect(report).toEqual({
      moved: { media: [{ media: "youtube", from: "td" }] },
      dropped: {
        hiddenRows: 2,
        hiddenCells: 3,
        hiddenOther: 1,
        fontFamilies: ["Arial", "Calibri"],
        frames: 4,
        youtubeRefusals: ["video_id", "list_type"],
        embedParams: ["controls", "playlist"],
      },
    })
    // The report is a copy: the draft can go on without changing it.
    draft.moved.push({ media: "image", from: "li" })
    expect(report.moved.media).toHaveLength(1)
  })

  it("lists every refusal and dropped option once, in its fixed order", () => {
    fc.assert(
      fc.property(
        fc.array(fc.constantFrom(...YOUTUBE_REFUSALS)),
        fc.array(fc.constantFrom(...DROPPED_OPTIONS)),
        (refusals, options) => {
          const draft = emptyReportDraft()
          refusals.forEach((reason) => draft.youtubeRefusals.add(reason))
          options.forEach((option) => draft.embedParams.add(option))
          const { dropped } = finishReport(draft)
          expect(dropped.youtubeRefusals).toEqual(
            YOUTUBE_REFUSALS.filter((reason) => refusals.includes(reason))
          )
          expect(dropped.embedParams).toEqual(
            DROPPED_OPTIONS.filter((option) => options.includes(option))
          )
        }
      )
    )
  })
})
