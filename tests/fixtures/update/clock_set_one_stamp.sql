-- An `UPDATE` stamping two columns from `now()` over many rows writes one instant: the
-- two columns of a row are equal and the rows are equal. `SET … = DEFAULT` takes the
-- same instant the statement's own `now()` does.
-- @setup
CREATE TABLE cus_d (x INT);
INSERT INTO cus_d VALUES (0), (1), (2), (3), (4), (5), (6), (7);
CREATE TABLE cus (id INT, a TIMESTAMPTZ, b TIMESTAMPTZ, c TIMESTAMPTZ DEFAULT now(), s TIMESTAMPTZ);
INSERT INTO cus (id) SELECT a.x * 64 + b.x * 8 + c.x FROM cus_d a, cus_d b, cus_d c;
UPDATE cus SET a = now(), b = CURRENT_TIMESTAMP, c = DEFAULT, s = statement_timestamp();
-- @test
SELECT count(*) AS updated_rows,
       count(DISTINCT a) AS a_values,
       count(DISTINCT s) AS statement_values,
       sum(CASE WHEN a = b AND a = c THEN 0 ELSE 1 END) AS columns_differing,
       sum(CASE WHEN a = now() THEN 0 ELSE 1 END) AS not_this_transaction
FROM cus;
