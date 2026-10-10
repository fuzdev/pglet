-- The over-refusal control for the rule above, and the reason it is keyed on the
-- **elements** rather than on the node: `ARRAY['ok'::mood]` is an `Expr::Array` too,
-- its identity is `Undecided` for the reason a written `x::mood` is (`EDGES.md` row
-- 104 — the cast parses to a cast to `text`), and 18.6 types *that* array `mood[]` and
-- resolves the operator. Folding every `Expr::Array` to `text` refuses this.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
SELECT id,
       (m > ANY(ARRAY['ok'::mood, 'sad'])) AS mixed,
       (m = ANY(ARRAY[m])) AS self_array,
       (m = ANY(ARRAY[m, 'ok'])) AS member_beside_column
FROM w ORDER BY id;
