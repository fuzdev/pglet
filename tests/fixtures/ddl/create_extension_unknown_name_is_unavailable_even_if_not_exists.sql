-- `IF NOT EXISTS` does not excuse an unavailable extension: it suppresses the
-- *already exists* error, not the *no such extension* one. Measured on 18.6, and the
-- reason the name check runs ahead of the existence question here too.
--
-- **A lock, not a new assertion** — green against an engine that refuses the whole
-- statement, for the same reason as its sibling: the blanket `0A000` is the code 18.6
-- answers with here.
-- @test
-- @expect-error: extension "nosuch_ext" is not available
CREATE EXTENSION IF NOT EXISTS nosuch_ext;
