-- The multi-word and modifier-bearing type names. These are the shapes two tokens of
-- lookahead cannot decide — every lead word here (`double`, `character`, `numeric`,
-- `varchar`, `timestamp`) is also a legal bare column name on both engines — so the
-- parser reaches them through a speculative parse that restores the cursor when no
-- string literal follows the type name.
-- @test
SELECT
  DOUBLE PRECISION '1.5' AS dp,
  CHARACTER VARYING 'ab' AS cv,
  CHARACTER VARYING(3) 'abcd' AS cv3,
  NUMERIC(5,2) '1.5' AS n52,
  NUMERIC(3,1) '1.55' AS n31,
  VARCHAR(3) 'abcd' AS v3;
