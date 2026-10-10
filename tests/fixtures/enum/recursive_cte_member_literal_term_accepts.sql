-- Carrying the seed's enum into `check_recursive_shape`'s fold must not make that fold
-- **stricter** than the OID fold it replaced. 18.6 does not fold an `unknown` recursive
-- entry against the seed's type — it converts it, running the enum's input function — so
-- a member literal in the recursive term is two rows there. Reading the identity on that
-- branch refused it `42804 UNION types mood and text cannot be matched`, measured, which
-- is a loud over-refusal traded for a silent residual; the `unknown` branch folds by the
-- OIDs alone for that reason.
--
-- The conversion runs after the fold, as 18.6's does: a **non**-member literal in the
-- same position is `22P02` on both engines, and
-- `with_recursive_list/enum_recursive_term_non_member_literal_loud.sql` holds it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'ok'), (2, 'happy');

-- @test
WITH RECURSIVE r(x) AS (
  SELECT m FROM t
  UNION ALL
  SELECT 'sad' FROM r WHERE false
)
SELECT count(*) AS n FROM r;
