-- INTERVAL `postgres`-style output: signed pluralization (`-1` → `-1 years`,
-- plural), the `+`-forced-on-positive-after-negative rule, `mon`/`mons` (not
-- `month`), and the zero interval → `00:00:00`.
-- @test
SELECT INTERVAL '-14 months -3 days -4:05:06' AS all_neg,
       INTERVAL '-14 months 3 days -4:05:06'  AS mixed_sign,
       INTERVAL '1 year'                      AS singular,
       INTERVAL '-1 year'                     AS neg_plural,
       INTERVAL '0'                           AS zero;
