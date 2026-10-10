-- The accept side of `quoted_public_schema_qualifier_loud.sql`, in the identifier
-- position: an unquoted qualifier folds, so `public`, `PUBLIC` and the exact quoted
-- `"public"` all name the implicit schema. A repair that simply stopped comparing the
-- qualifier case-insensitively would satisfy the reject side and redden this one.
-- @setup
CREATE TABLE upsq (a INT);
INSERT INTO upsq VALUES (1);

-- @test
SELECT (SELECT count(*) FROM public.upsq) AS bare,
       (SELECT count(*) FROM PUBLIC.upsq) AS upper,
       (SELECT count(*) FROM "public".upsq) AS quoted;
