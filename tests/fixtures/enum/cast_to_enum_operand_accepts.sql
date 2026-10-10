-- A written `x::mood` reaches the planner as a cast to `text` over the membership test
-- (`StringFunc::EnumIn`), which carries the enum's name for that test alone — so the gates
-- read it as they read `x::text`, and its identity is **undecidable** rather than absent. Refusing it against an enum would break
-- this, which 18.6 answers; the cost of not refusing is the `m = s::text` under-refusal
-- `EDGES.md` row 114 records as its cast-undecidable residual (the erasure is row 104's).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, n mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
SELECT id, greatest(m, s::mood)::text AS g, least(m, s::mood)::text AS l,
       COALESCE(m, s::mood)::text AS c, (m > s::mood) AS gt
  FROM t ORDER BY id;
