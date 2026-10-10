-- The over-refusal control for the three fixtures above. An `unknown` element beside a
-- typed one adopts that element's type, so an array whose typed element **is** the enum
-- resolves the operator — for a member label, for a `NULL`, and for the cast whose enum
-- name the parser dropped (`EDGES.md` row 104), which leaves the array undecidable and
-- so falls back to the OID rule rather than refusing.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
SELECT id,
       (m = ANY(ARRAY[m, 'ok'])) AS member_beside_column,
       (m = ANY(ARRAY[m, NULL])) AS null_beside_column,
       (m = ANY(ARRAY['ok'::mood, NULL])) AS null_beside_cast,
       (ARRAY[m] = ARRAY['ok'::mood]) AS cast_element_pair
FROM w ORDER BY id;
