-- @setup
CREATE TABLE labels (id INT, txt TEXT);
INSERT INTO labels VALUES (1, '50% off'), (2, '50xx off'), (3, 'a_b'), (4, 'axb');

-- @test
-- `\` is the default LIKE escape (Postgres-compatible): `\%` matches a literal
-- `%` and `\_` a literal `_`, so the wildcard meaning is suppressed. An explicit
-- `ESCAPE '\'` is accepted as the (redundant) default — the spine's escaped
-- prefix-search shape.
SELECT id, txt FROM labels
WHERE txt LIKE '50\%%' ESCAPE '\'
   OR txt LIKE 'a\_b'
ORDER BY id ASC;
