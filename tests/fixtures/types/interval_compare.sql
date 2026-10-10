-- INTERVAL comparison uses the Postgres estimate (`micros + (months*30 + days) *
-- 86_400_000_000`), so `30 days` equals `1 mon` and ordering mixes days/months.
-- @test
SELECT INTERVAL '30 days' = INTERVAL '1 mon'   AS thirty_eq_mon,
       INTERVAL '30 days' < INTERVAL '2 mons'  AS lt,
       INTERVAL '1 mon' > INTERVAL '10 days'   AS gt,
       INTERVAL '1 mon' <> INTERVAL '31 days'  AS ne;
