-- The window path evaluates its non-window projections through its own scope, so
-- it needs the declared column types as well — otherwise `'v=' || a` beside a
-- `row_number()` drops the `timestamptz` `+00` offset that the same expression
-- keeps in a plain projection.
--
-- `id` is projected because a trailing `ORDER BY` above a Window node can only see
-- the window output — unlike a plain projection, which the planner sorts *below*
-- when a key names a dropped base column. That gap is unrelated to this fixture,
-- so it is avoided rather than exercised.
-- @setup
CREATE TABLE ctzw (id INT, a TIMESTAMPTZ);
INSERT INTO ctzw VALUES (1, '2026-07-18 12:30:45+00'), (2, '2026-07-19 01:02:03+00');
-- @test
SELECT id, 'v=' || a AS tz_concat, a::text AS tz_cast, row_number() OVER (ORDER BY id) AS rn
FROM ctzw
ORDER BY id;
