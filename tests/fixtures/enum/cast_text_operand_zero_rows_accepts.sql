-- The accept-side control that says the `text`-operand conversion is an **evaluation**:
-- over no row 18.6 converts nothing and answers `(0 rows)`, so a plan-time check of a
-- non-literal operand would over-refuse here.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
SELECT 'q'::text::mood::text AS v FROM t WHERE false;
