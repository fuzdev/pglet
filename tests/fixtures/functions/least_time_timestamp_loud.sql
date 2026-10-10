-- `least`'s half of the same fold — locked separately because the two take different
-- branches through the aggregate-style comparison and only one was exercised.
-- @setup
CREATE TABLE ltt (tm TIME, ts TIMESTAMP);
-- NULL `time`, for the same reason as the `greatest` fixture: a NULL argument is
-- dropped by the eval-time fold, so this row reaches the hazard the previous seed
-- (both columns non-NULL) walked straight past.
INSERT INTO ltt VALUES (NULL, '2026-01-01 10:00:00');
-- @test
-- @expect-error: no common type for time and timestamp (42846)
SELECT least(tm, ts) FROM ltt;
