"""Provider-neutral agent example. The broker, not this process, owns policy decisions."""

import os

from agent_treasury import TreasuryClient


client = TreasuryClient(
    socket_path=os.environ["TREASURY_SOCKET_PATH"],
    token_file=os.environ["TREASURY_AGENT_TOKEN_FILE"],
)
print(client.get_budget())
print(client.get_receive_instructions())

