#!/usr/bin/env python3
"""Select an explicit CLI mode; never enroll a ranker or manufacture a producer pin."""
import sys
import tomllib
from pathlib import Path


def main():
    try:
        policy = tomllib.loads(Path(sys.argv[1]).read_text())
    except (OSError, ValueError, IndexError) as error:
        print(f"judge mode: cannot read policy: {error}", file=sys.stderr)
        return 4
    # Presence commits the caller to authority admission: malformed, empty or mismatched
    # allowlists are the judge's refusal, never a reason to downgrade silently.
    print("authority" if "producer" in policy else "advisory")
    return 0


if __name__ == "__main__":
    sys.exit(main())
