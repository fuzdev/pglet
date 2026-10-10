-- upper / length / replace over text rows, including a NULL row, an empty string,
-- and a multibyte value (length counts characters, not bytes).
-- @setup
CREATE TABLE s (id INT, t TEXT);
INSERT INTO s VALUES (1, 'abc'), (2, 'josé'), (3, ''), (4, NULL), (5, 'xxaxxbxx');
-- @test
SELECT id,
  upper(t) AS up,
  length(t) AS len,
  replace(t, 'xx', 'y') AS rep
FROM s ORDER BY id;
