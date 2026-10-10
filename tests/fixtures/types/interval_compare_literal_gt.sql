-- A range comparison of an INTERVAL column against a string literal coerces the
-- literal to interval (not a text compare), so `span > '15 days'` returns the rows
-- whose estimated interval exceeds 15 days.
-- @setup
CREATE TABLE iv_cmp (id INT, span INTERVAL);
INSERT INTO iv_cmp VALUES (1, INTERVAL '1 mon'), (2, INTERVAL '2 mons'),
                          (3, INTERVAL '30 days'), (4, INTERVAL '10 days');
-- @test
SELECT id, span FROM iv_cmp WHERE span > '15 days' ORDER BY id;
