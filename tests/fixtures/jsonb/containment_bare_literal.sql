-- `@>` with an **uncast** literal — the spelling Postgres's own documentation uses
-- (`WHERE data @> '{"k":1}'`), and the one pglet rejected.
--
-- Postgres's containment operators are `jsonb @> jsonb` and `anyarray @> anyarray`,
-- so an unknown-typed literal resolves to the other operand's type. pglet's
-- plan-time coercion pass covered comparison operators but not containment, so the
-- literal stayed `text` and the query failed loud with "containment is only
-- supported for arrays and JSONB objects" — an over-rejection that forced an
-- explicit `::jsonb` on every containment predicate. Both container families are
-- covered here, since one rule serves them.
-- @setup
CREATE TABLE jcbl (id INT, d JSONB, tags TEXT[]);
INSERT INTO jcbl VALUES
  (1, '{"n":1,"tags":["x"]}', '{a,b}'),
  (2, '{"n":2}', '{b}'),
  (3, '[1,2,3]', '{a}');
-- @test
SELECT id FROM jcbl WHERE d @> '{"n":1}' OR d @> '[1,3]' OR tags @> '{a}' ORDER BY id;
