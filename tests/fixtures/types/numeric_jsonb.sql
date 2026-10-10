-- A NUMERIC cast into JSONB (jsonb_build_object) and text.
-- @setup
CREATE TABLE t (id INT, price NUMERIC(10,2));
INSERT INTO t VALUES (1, 19.99);
-- @test
SELECT jsonb_build_object('price', price::text) AS obj FROM t;
