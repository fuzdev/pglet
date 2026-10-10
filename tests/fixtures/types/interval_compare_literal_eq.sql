-- A bare string literal compared against an INTERVAL column is coerced to interval
-- (previously never coerced — it text-compared and returned wrong rows). Uses the
-- Postgres estimate, so `30 days` = `1 mon`: `span = '1 mon'` matches both.
-- @setup
CREATE TABLE iv_cmp (id INT, span INTERVAL);
INSERT INTO iv_cmp VALUES (1, INTERVAL '1 mon'), (2, INTERVAL '2 mons'),
                          (3, INTERVAL '30 days'), (4, INTERVAL '10 days');
-- @test
SELECT id, span FROM iv_cmp WHERE span = '1 mon' ORDER BY id;
