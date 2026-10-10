-- Numeric sqrt at table-maker's-dilemma boundaries: Newton with a fixed guard
-- scale rounds the last digit the wrong way here; the exact ½-ulp correction
-- lands on Postgres's value (…000 and …002, not …001 and …003).
-- @test
SELECT
  sqrt(1.000000000000001) AS a,
  sqrt(4.00000000000001) AS b,
  sqrt(0.25) AS c;
