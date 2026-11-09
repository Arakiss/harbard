// The robot models: ids, order and screen boxes. Generated with the artwork;
// do not edit by hand.

/** Pixel sizes rendered for every body and mask, smallest first. */
export const ROBOT_SIZES = [36, 72, 144, 216, 432] as const;

export interface RobotModel {
  id: string;
  /** The screen's bounding box, as fractions of the tile. */
  screen: { x: number; y: number; w: number; h: number };
}

/** In order: the name hash indexes into this list. */
export const ROBOT_MODELS: readonly RobotModel[] = [
  {
    "id": "e-off",
    "screen": {
      "x": 0.10367,
      "y": 0.18581,
      "w": 0.52153,
      "h": 0.41866
    }
  },
  {
    "id": "antenna",
    "screen": {
      "x": 0.12679,
      "y": 0.22488,
      "w": 0.49841,
      "h": 0.38836
    }
  },
  {
    "id": "dome",
    "screen": {
      "x": 0.10686,
      "y": 0.18979,
      "w": 0.51834,
      "h": 0.41308
    }
  },
  {
    "id": "dials",
    "screen": {
      "x": 0.10447,
      "y": 0.18581,
      "w": 0.52073,
      "h": 0.39553
    }
  },
  {
    "id": "radio",
    "screen": {
      "x": 0.10367,
      "y": 0.1866,
      "w": 0.52153,
      "h": 0.41707
    }
  },
  {
    "id": "teapot",
    "screen": {
      "x": 0.13078,
      "y": 0.20893,
      "w": 0.49442,
      "h": 0.38995
    }
  },
  {
    "id": "headset",
    "screen": {
      "x": 0.10845,
      "y": 0.1866,
      "w": 0.51675,
      "h": 0.41707
    }
  },
  {
    "id": "propeller",
    "screen": {
      "x": 0.10526,
      "y": 0.19617,
      "w": 0.51754,
      "h": 0.40829
    }
  },
  {
    "id": "lantern",
    "screen": {
      "x": 0.10686,
      "y": 0.24083,
      "w": 0.50877,
      "h": 0.37719
    }
  },
  {
    "id": "bowler",
    "screen": {
      "x": 0.10447,
      "y": 0.18501,
      "w": 0.52073,
      "h": 0.41946
    }
  },
  {
    "id": "flower",
    "screen": {
      "x": 0.10367,
      "y": 0.18581,
      "w": 0.52153,
      "h": 0.41866
    }
  },
  {
    "id": "stripes",
    "screen": {
      "x": 0.10447,
      "y": 0.18581,
      "w": 0.52073,
      "h": 0.41866
    }
  }
];
