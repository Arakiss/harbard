/**
 * The console shows times on the machine's own clock, so a test that asserts
 * "22:15" or which day a timestamp falls on would pass or fail by where it
 * runs. The zone is pinned once, here, before any module reads it, instead of
 * setting it test by test.
 */
process.env.TZ = "UTC";
