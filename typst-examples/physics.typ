// author: sjfh (https://github.com/sjfhsjfh)
#import "@preview/oxifmt:1.0.0": strfmt
#set page(width: auto, height: auto, margin: 1cm)

Physics rendered using `typst`!

#let FRAME_COUNT = 720
#let t = int(sys.inputs.at("t", default: FRAME_COUNT))
#let FPS = 24.0
#let TPS = 180.0
#let dt = 1.0 / TPS

#let data = ()

#let cur-x = -0.5
#let cur-y = 0.5
#let cur-vx = 1.0
#let cur-vy = 0.2
#let g = 4 // 5m x 5m block ~ 2 * 2 block => 10m/s^2 ~ 4
#let THRESHOLD = 0.01
#let recov-e = 0.9
#let shadow-tau = 0.25 // seconds

#for _t in range(0, int(FRAME_COUNT * TPS / FPS) + 1) {
  cur-vy = cur-vy - g * dt
  if (cur-y + cur-vy * dt < -1.0 - THRESHOLD) {
    cur-y = -1.0
    cur-vy = -cur-vy * recov-e
  }
  if (cur-x + cur-vx * dt > 1.0 + THRESHOLD) {
    cur-x = 1.0
    cur-vx = -cur-vx * recov-e
  }
  if (cur-x + cur-vx * dt < -1.0 - THRESHOLD) {
    cur-x = -1.0
    cur-vx = -cur-vx * recov-e
  }
  cur-x = cur-x + cur-vx * dt
  cur-y = cur-y + cur-vy * dt
  data.push((cur-x, cur-y))
}

#figure(
  block(
    width: 100pt,
    height: 100pt,
    stroke: 0.5pt,
    {
      for (cur, (x, y)) in data.slice(0, int(t / FPS * TPS)).enumerate() {
        place(
          center + horizon,
          circle(
            fill: black.transparentize(
              100% * (1.0 - calc.exp(-(t / FPS * TPS - cur) / TPS / shadow-tau)),
            ),
            radius: 1pt,
          ),
          dx: 50% * x,
          dy: 50% * -y,
        )
      }
    },
  ),
  caption: [Time: $#strfmt("{:.2}", t / TPS) #h(1em / 6) "s"$],
  numbering: none,
)