-- An unrecognized statement keyword — a typo or garbage leading token (`SELEC`,
-- not `SELECT`) — is a `42601` syntax error at that token, matching Postgres, not
-- the `0A000` "unsupported construct" (that stays only for a *recognized* statement
-- kind pglet deliberately doesn't implement).
-- @test
-- @expect-error: garbage leading token is a syntax error (42601), not 0A000
SELEC 1;
