const fs = require('fs');
const path = require('path');

const host = 'localhost';

module.exports = {
  mode: "production",
  devtool: "source-map",
  resolve: { extensions: [".jsx", ".js"] },
  module: {
    rules: [
      {
        test: /\.m?jsx?$/,
        exclude: /(node_modules|bower_components)/,
        loader: 'babel-loader',
        options: {
          presets: [
            '@babel/preset-env',
            '@babel/preset-react',
            {
              plugins: ['@babel/plugin-proposal-class-properties']
            }
          ]
        }
      },
      {
        enforce: "pre",
        test: /\.js$/,
        loader: "source-map-loader"
      },
      {
        test: /\.(png|svg|jpg|gif)$/,
        use: [
          'file-loader',
        ],
      },
    ]
  },
  externals: {
    "react": "React",
    "react-dom": "ReactDOM"
  },
  optimization: {
    minimize: false
  },
  output: {
    path: path.resolve(__dirname, 'dist'),
    publicPath: '/dist/',
    filename: 'main.js'
  },
  devServer: {
    host,
    client: {
      overlay: {
        warnings: false,
        errors: false,
      },
    },
    static: {
      directory: path.join(__dirname, '.'),
      serveIndex: true,
    },
    server: {
      type: 'https',
      options: {
        key: fs.readFileSync('../server/certs/server.key'),
        cert: fs.readFileSync('../server/certs/server.crt'),
        ca: fs.readFileSync('../server/certs/rootCA.pem'),
      },
    },
    proxy: {
      '/wss': {
        target: `http://${host}:7070`,
        secure: true,
        ws: true
      },

      '/api': {
        target: `https://${host}:7070`,
        secure: false,
        changeOrigin: true,
        pathRewrite: {'^/api' : ''}
      }
    }
  }
};
