-- `sum(bigint)` accumulates exactly, above 2^53: 2^53 + 2^53 + 1 = 2^54 + 1 is not
-- representable in f64 (it rounds to 2^54), so an f64-accumulating `sum` returned
-- 18014398509481984. It is `numeric`, as in 18.6, on both legs.
-- @setup
CREATE TABLE big_sum (v BIGINT);
INSERT INTO big_sum VALUES (9007199254740992), (9007199254740992), (1);
-- @test
SELECT SUM(v) AS s FROM big_sum;
