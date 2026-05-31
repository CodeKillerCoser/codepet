type LottiePoint = [number, number];

type LottiePath = {
  c: boolean;
  i: LottiePoint[];
  o: LottiePoint[];
  v: LottiePoint[];
};

function openPath(v: LottiePoint[], i: LottiePoint[], o: LottiePoint[]): LottiePath {
  return { c: false, i, o, v };
}

function animatedPath(frames: Array<{ t: number; shape: LottiePath }>) {
  return {
    a: 1,
    k: frames.map((frame, index) => {
      const next = frames[index + 1];
      if (!next) {
        return { t: frame.t, s: [frame.shape] };
      }
      return {
        t: frame.t,
        s: [frame.shape],
        e: [next.shape],
        i: { x: [0.38], y: [1] },
        o: { x: [0.16], y: [0] },
      };
    }),
    ix: 2,
  };
}

function staticPath(shape: LottiePath) {
  return {
    a: 0,
    k: shape,
    ix: 2,
  };
}

function opacityKeyframes(values: Array<{ t: number; value: number }>) {
  return {
    a: 1,
    k: values.map((frame, index) => {
      const next = values[index + 1];
      if (!next) {
        return { t: frame.t, s: [frame.value] };
      }
      return {
        t: frame.t,
        s: [frame.value],
        e: [next.value],
        i: { x: [0.4], y: [1] },
        o: { x: [0.2], y: [0] },
      };
    }),
    ix: 11,
  };
}

function layerTransform(opacity = 100) {
  return {
    a: { a: 0, k: [0, 0, 0], ix: 1 },
    o: typeof opacity === "number" ? { a: 0, k: opacity, ix: 11 } : opacity,
    p: { a: 0, k: [0, 0, 0], ix: 2 },
    r: { a: 0, k: 0, ix: 10 },
    s: { a: 0, k: [100, 100, 100], ix: 6 },
  };
}

function stroke(color: [number, number, number, number], width: number, opacity = 100) {
  return {
    ty: "st",
    c: { a: 0, k: color, ix: 3 },
    o: { a: 0, k: opacity, ix: 4 },
    w: { a: 0, k: width, ix: 5 },
    lc: 2,
    lj: 2,
    ml: 4,
    bm: 0,
    nm: "Stroke",
  };
}

const cordFrames = [
  {
    t: 0,
    shape: openPath(
      [[190, 35], [154, 39], [118, 57], [86, 95], [70, 132]],
      [[0, 0], [9, -7], [18, -5], [10, -25], [9, -11]],
      [[-12, 4], [-16, 12], [-19, 6], [-9, 24], [0, 0]],
    ),
  },
  {
    t: 5,
    shape: openPath(
      [[190, 35], [158, 48], [108, 50], [58, 70], [34, 106]],
      [[0, 0], [7, -11], [19, 9], [20, -20], [9, -13]],
      [[-11, 8], [-21, 16], [-21, -10], [-12, 12], [0, 0]],
    ),
  },
  {
    t: 10,
    shape: openPath(
      [[190, 35], [142, 39], [93, 78], [51, 121], [30, 128]],
      [[0, 0], [14, -10], [21, -7], [12, -25], [8, 1]],
      [[-17, 4], [-24, 12], [-19, 8], [-9, 15], [0, 0]],
    ),
  },
  {
    t: 15,
    shape: openPath(
      [[190, 35], [148, 57], [111, 100], [71, 118], [38, 98]],
      [[0, 0], [9, -17], [5, -19], [22, -3], [12, 11]],
      [[-15, 12], [-11, 22], [-9, 24], [-18, 2], [0, 0]],
    ),
  },
  {
    t: 24,
    shape: openPath(
      [[190, 35], [160, 44], [128, 61], [96, 94], [78, 126]],
      [[0, 0], [7, -7], [15, -4], [8, -21], [8, -10]],
      [[-11, 5], [-14, 11], [-17, 5], [-9, 20], [0, 0]],
    ),
  },
];

const flashPaths = [
  openPath([[27, 82], [55, 111]], [[0, 0], [-10, -9]], [[10, 10], [0, 0]]),
  openPath([[53, 84], [29, 112]], [[0, 0], [10, -10]], [[-8, 10], [0, 0]]),
  openPath([[41, 75], [41, 120]], [[0, 0], [0, -12]], [[0, 13], [0, 0]]),
  openPath([[19, 98], [64, 98]], [[0, 0], [-14, 0]], [[14, 0], [0, 0]]),
];

export const whipCrackAnimation = {
  v: "5.12.0",
  fr: 36,
  ip: 0,
  op: 26,
  w: 220,
  h: 150,
  nm: "Code Pet Whip Crack",
  ddd: 0,
  assets: [],
  layers: [
    {
      ddd: 0,
      ind: 1,
      ty: 4,
      nm: "Whip Shadow",
      sr: 1,
      ks: layerTransform(opacityKeyframes([{ t: 0, value: 0 }, { t: 2, value: 38 }, { t: 19, value: 30 }, { t: 26, value: 0 }])),
      ao: 0,
      shapes: [
        {
          ty: "gr",
          nm: "Shadow Group",
          it: [
            { ty: "sh", nm: "Shadow Curve", ks: animatedPath(cordFrames) },
            stroke([0.05, 0.07, 0.12, 1], 9, 22),
            { ty: "tr", p: { a: 0, k: [0, 0], ix: 2 }, a: { a: 0, k: [0, 0], ix: 1 }, s: { a: 0, k: [100, 100], ix: 3 }, r: { a: 0, k: 0, ix: 6 }, o: { a: 0, k: 100, ix: 7 } },
          ],
        },
      ],
      ip: 0,
      op: 26,
      st: 0,
      bm: 0,
    },
    {
      ddd: 0,
      ind: 2,
      ty: 4,
      nm: "Whip Cord",
      sr: 1,
      ks: layerTransform(opacityKeyframes([{ t: 0, value: 0 }, { t: 2, value: 100 }, { t: 20, value: 100 }, { t: 26, value: 0 }])),
      ao: 0,
      shapes: [
        {
          ty: "gr",
          nm: "Cord Group",
          it: [
            { ty: "sh", nm: "Cord Curve", ks: animatedPath(cordFrames) },
            stroke([0.19, 0.13, 0.08, 1], 4, 100),
            { ty: "tr", p: { a: 0, k: [0, 0], ix: 2 }, a: { a: 0, k: [0, 0], ix: 1 }, s: { a: 0, k: [100, 100], ix: 3 }, r: { a: 0, k: 0, ix: 6 }, o: { a: 0, k: 100, ix: 7 } },
          ],
        },
      ],
      ip: 0,
      op: 26,
      st: 0,
      bm: 0,
    },
    {
      ddd: 0,
      ind: 3,
      ty: 4,
      nm: "Crack Flash",
      sr: 1,
      ks: layerTransform(opacityKeyframes([{ t: 0, value: 0 }, { t: 8, value: 0 }, { t: 10, value: 100 }, { t: 16, value: 0 }, { t: 26, value: 0 }])),
      ao: 0,
      shapes: [
        {
          ty: "gr",
          nm: "Flash Group",
          it: [
            ...flashPaths.map((shape, index) => ({ ty: "sh", nm: `Flash Ray ${index + 1}`, ks: staticPath(shape) })),
            stroke([0.98, 0.78, 0.09, 1], 5, 100),
            { ty: "tr", p: { a: 0, k: [0, 0], ix: 2 }, a: { a: 0, k: [0, 0], ix: 1 }, s: { a: 0, k: [100, 100], ix: 3 }, r: { a: 0, k: 0, ix: 6 }, o: { a: 0, k: 100, ix: 7 } },
          ],
        },
      ],
      ip: 0,
      op: 26,
      st: 0,
      bm: 0,
    },
  ],
};
