-- A bare string literal in a `WHERE` is Postgres's `unknown`, resolved at this
-- position through the boolean input function — so every spelling that function
-- takes is a row, and `'f'`/`'0'` are not. Answered zero rows for all of them
-- before the boolean-context resolver landed, `'t'` included.
-- @setup
CREATE TABLE bcw (a INT);
INSERT INTO bcw VALUES (1);
-- @test
SELECT
  (SELECT count(*) FROM bcw WHERE 't') AS t_spelling,
  (SELECT count(*) FROM bcw WHERE 'true') AS true_spelling,
  (SELECT count(*) FROM bcw WHERE 'yes') AS yes_spelling,
  (SELECT count(*) FROM bcw WHERE '1') AS one_spelling,
  (SELECT count(*) FROM bcw WHERE 'on') AS on_spelling,
  (SELECT count(*) FROM bcw WHERE 'f') AS f_spelling,
  (SELECT count(*) FROM bcw WHERE '0') AS zero_spelling;
