"""Driver selection for the primary application database."""


def normalize_database_url(url: str) -> str:
    """Select the declared psycopg2 driver for bare PostgreSQL URLs.

    SQLAlchemy 2.1 defaults to psycopg 3, which is not a dependency here.
    Replace only the scheme so encoded credentials and query strings remain
    byte-for-byte intact. Explicit drivers and other backends are unchanged.
    """
    prefix = "postgresql://"
    if url.startswith(prefix):
        return "postgresql+psycopg2://" + url[len(prefix):]
    return url
