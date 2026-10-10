-- `CURRENT_DATE` — the SQL value function (parenless), the current UTC date. Its
-- value is non-deterministic, so compare only deterministic derived properties:
-- `CURRENT_DATE - CURRENT_DATE = 0` (also exercises `date - date`), equality with
-- `now()::date` (cross-checks against the timestamp clock), and a fixed lower
-- bound. All are identical on pglet and Postgres at TimeZone=UTC.
-- @test
SELECT CURRENT_DATE - CURRENT_DATE       AS zero,
       CURRENT_DATE = now()::date        AS matches_now,
       CURRENT_DATE >= '2020-01-01'::date AS after_2020;
