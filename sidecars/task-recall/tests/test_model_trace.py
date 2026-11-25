import asyncio
import unittest
import uuid


class TraceTests(unittest.IsolatedAsyncioTestCase):
    async def test_parallel_contexts_and_unscoped_search_cannot_borrow_attempt_identity(self):
        import httpx
        import httpx2
        from codepet_recall import model_trace
        originals = [(module, module.AsyncClient.send) for module in (httpx, httpx2)]
        observed = []
        async def handle(request):
            await asyncio.sleep(0)
            observed.append((str(request.url), request.headers.get('X-CodePet-Processing-Job')))
            return httpx.Response(200, json={})
        model_trace.install('http://127.0.0.1:49123/v1', 'private')
        try:
            async with httpx.AsyncClient(transport=httpx.MockTransport(handle)) as client:
                async def call(job, url, key='private'):
                    token = model_trace.attempt_id.set(job)
                    try:
                        await client.post(url, headers={'Authorization':'Bearer '+key})
                    finally:
                        model_trace.attempt_id.reset(token)
                first, second = str(uuid.uuid4()), str(uuid.uuid4())
                endpoint = 'http://127.0.0.1:49123/v1/embeddings'
                await asyncio.gather(call(first, endpoint), call(second, endpoint), call(None, endpoint),
                                     call(first, 'https://upstream.invalid/v1/embeddings'), call(first, endpoint, 'wrong'))
                self.assertEqual([item[1] for item in observed], [first, second, None, None, None])
            # Exercise the actual OpenAI SDK path, not just a bare legacy httpx client.
            from openai import AsyncOpenAI
            async def embedding(request):
                observed.append((str(request.url), request.headers.get('X-CodePet-Processing-Job')))
                return httpx2.Response(200, json={'data':[{'index':0,'embedding':[1.0,2.0]}], 'model':'model', 'object':'list', 'usage':{'prompt_tokens':1,'total_tokens':1}})
            async with httpx2.AsyncClient(transport=httpx2.MockTransport(embedding)) as native:
                sdk = AsyncOpenAI(base_url='http://127.0.0.1:49123/v1', api_key='private', http_client=native)
                scope = model_trace.attempt_id.set(first)
                try:
                    await sdk.embeddings.create(model='model', input=['text'])
                finally:
                    model_trace.attempt_id.reset(scope)
                await sdk.embeddings.create(model='model', input=['search'])
                self.assertEqual(observed[-2][1], first)
                self.assertIsNone(observed[-1][1])
        finally:
            for module, original in originals:
                module.AsyncClient.send = original
            model_trace._installed = False
