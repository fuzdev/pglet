-- The DESC leg of the GIN-serves-an-ordered-scan bug. Worth its own fixture
-- because the ASC and DESC answers are independent: the buggy ASC reply for two
-- rows is a palindrome (`1,2,1`), so a two-row DESC fixture would have agreed with
-- ASC and proved nothing. Three rows separate them — Postgres answers `3,1,2` DESC
-- against `2,1,3` ASC, and the buggy build answered `1,2,3,3,3,1` against
-- `1,3,3,3,2,1`.
-- Not `-- @profile-lock` — see `gin_method_order_by_jsonb.sql`.
-- @setup
CREATE TABLE gin_ord_jd (id int4, j jsonb NOT NULL);
INSERT INTO gin_ord_jd (id, j) VALUES
  (1, '{"z":1,"a":2}'),
  (2, '{"m":3}'),
  (3, '{"b":1,"c":2,"d":3}');
CREATE INDEX gin_ord_jd_idx ON gin_ord_jd USING gin(j);
-- @test
SELECT id, j FROM gin_ord_jd ORDER BY j DESC;
