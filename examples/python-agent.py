"""Provider-neutral agent example. The broker, not this process, owns policy decisions."""

import os

from cixa import CixaClient


client = CixaClient(
    socket_path=os.environ["CIXA_SOCKET_PATH"],
    token_file=os.environ["CIXA_AGENT_TOKEN_FILE"],
)
print(client.get_budget())
print(client.get_receive_instructions())

