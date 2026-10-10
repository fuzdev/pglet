-- An `IN` list of string literals against an INTERVAL column coerces each element to
-- interval. `30 days` = `1 mon` (Postgres estimate), so row 3 also matches.
-- @setup
CREATE TABLE iv_cmp (id INT, span INTERVAL);
INSERT INTO iv_cmp VALUES (1, INTERVAL '1 mon'), (2, INTERVAL '2 mons'),
                          (3, INTERVAL '30 days'), (4, INTERVAL '10 days');
-- @test
SELECT id, span FROM iv_cmp WHERE span IN ('1 mon', '2 mons') ORDER BY id;
