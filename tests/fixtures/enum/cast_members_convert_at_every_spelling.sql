-- The member control: each spelling and operand kind that refuses a non-member converts a
-- member to its label — the literal, `CAST`, the typed constant, a `text` and a `varchar`
-- operand, a column, and `NULL`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE e (id INT, s TEXT);
INSERT INTO e VALUES (1, 'happy'), (2, NULL);

-- @test
SELECT id, 'ok'::mood::text AS lit, CAST('sad' AS mood)::text AS cst, (mood 'happy')::text AS tc,
       'ok'::text::mood::text AS txt, 'sad'::varchar::mood::text AS vc, s::mood::text AS col,
       NULL::mood::text AS nul
  FROM e ORDER BY id;
