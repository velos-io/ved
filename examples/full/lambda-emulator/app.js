exports.handler = async (event) => {
  return {
    statusCode: 200,
    headers: {
      'Content-Type': 'application/json'
    },
    body: JSON.stringify({
      status: 'ok',
      service: 'lambda-emulator',
      timestamp: new Date().toISOString()
    })
  };
};
