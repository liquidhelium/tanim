// author: ParaN3xus (https://github.com/ParaN3xus)

#let t = int(sys.inputs.at("t", default: 300))

#set page(width: 600pt, height: 300pt)
#set text(25pt)

#let manim-text(text, from, to, dur) = {
  if t < from {
    return hide(text)
  }
  if t > to {
    return text
  }

  let len = text.clusters().len()
  let stroke = 0.01em
  let fill = black
  let transparent = rgb(0, 0, 0, 0)

  text
    .clusters()
    .zip(range(len).map(x => from + x * (to - from - dur) / (len - 1)))
    .map(x => {
      let (c, from) = x
      let to = from + dur
      if t > to {
        return std.text(stroke: stroke, fill: fill, c)
      }
      if t < from {
        return hide(c)
      }

      let ratio = (t - from) / dur * 100%
      let conic-ratio = 100% - calc.min(100%, ratio * 2)

      let stroke = if ratio >= 50% {
        stroke
      } else {
        (
          stroke
            + gradient.conic(
              relative: "parent",
              angle: -90deg,
              (transparent, 0%),
              (transparent, conic-ratio),
              (fill, conic-ratio),
              (fill, 100%),
            )
        )
      }
      return {
        box(std.text(
          stroke: stroke,
          fill: fill.transparentize(calc.min(-2 * ratio + 200%)),
          c,
        ))
      }
    })
    .join()
}

#manim-text(
  `You're right, but "Genshin Impact" is a brand-new open-world adventure game independently developed by miHoYo. The game takes place in a fantasy world called "Teyvat," where those chosen by the gods are granted "Visions" to channel elemental powers. You will play as a mysterious character called the "Traveler," encountering companions with diverse personalities and unique abilities during your free journey. Together with them, you'll defeat powerful enemies and search for your lost sibling—while gradually uncovering the truth behind "Genshin Impact."
`.text,
  0,
  240,
  12,
)
