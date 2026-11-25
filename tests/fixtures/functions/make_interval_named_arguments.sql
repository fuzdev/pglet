-- `make_interval`'s arguments by name, with `=>` or `:=`, after positional ones, or none:
-- every unnamed field is zero.
-- @test
SELECT make_interval(hours => 2) AS h, make_interval(days => 1, mins => 30) AS dm,
       make_interval(years := 3) AS y, make_interval(1, hours => 2) AS mixed,
       make_interval(secs => 1.25) AS s, make_interval() AS zero,
       make_interval(weeks => 2, days => 3) AS wd, make_interval(years => -1, secs => -0.5) AS neg;
