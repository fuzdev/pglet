-- A searched `CASE WHEN`'s condition is a boolean context, so an unresolved
-- literal converts there too — the `THEN` arm is taken, not the `ELSE`.
-- @test
SELECT
  CASE WHEN 't' THEN 'y' ELSE 'n' END AS taken,
  CASE WHEN 'f' THEN 'y' ELSE 'n' END AS not_taken,
  CASE WHEN 'off' THEN 'y' ELSE 'n' END AS off_not_taken;
