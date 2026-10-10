-- The DEFAULT cook runs where 18.6 cooks a default, which is **behind** parse
-- analysis: an unresolvable column type is `42704` on the very statement whose other
-- column carries a bad default. Its mirror is `default_beats_a_duplicate_check_name`,
-- which the same bad default *wins*, so the pair is cross-code in both directions.
-- @expect-error: the column type is resolved before any default is cooked
-- @test
CREATE TABLE def_prec (a NOSUCHTYPE, b INT DEFAULT 'x');
