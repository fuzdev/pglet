-- Where the exponent cap is checked is part of the contract: 18.6 checks it **inside**
-- the exponent's digit run, so an exponent that stays inside the cap leaves the
-- trailing garbage to the syntax check and the answer is `22P02`, naming the literal.
-- Its pair — the same spelling behind an exponent past the cap — is
-- `arithmetic/numeric_cast_malformed_after_exponent_over_cap_loud.sql`, and the two
-- together pin the check's position rather than its existence.
-- @test
-- @expect-error: trailing garbage after an in-cap exponent (22P02)
-- @expect-message
SELECT '1e1073741823x'::numeric;
